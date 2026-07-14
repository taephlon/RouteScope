mod dns;
mod export;
mod geoip;
mod map;
mod probe;
mod stats;
mod traceroute;
mod tui;
mod util;
mod web_dashboard;

use clap::Parser;
use probe::ProbeMethod;
use stats::HopStats;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;
use traceroute::{run_traceroute, TraceConfig};

#[derive(Parser, Debug)]
#[command(
    name = "routescope",
    version = "0.1.0",
    about = "Modern MTR + GeoIP + ASN Lookup + Latency Visualizer & Web Dashboard",
    long_about = None
)]
struct Args {
    /// Target host or IP address to trace
    #[arg(required_unless_present = "web")]
    target: Option<String>,

    /// Use ICMP Echo request probes (default, requires cap_net_raw or root on Linux)
    #[arg(short = 'I', long)]
    icmp: bool,

    /// Use TCP SYN probes on the given port (default port 80)
    #[arg(short = 'T', long)]
    tcp: bool,

    /// Use UDP probes (unprivileged via MSG_ERRQUEUE)
    #[arg(short = 'U', long)]
    udp: bool,

    /// Number of probe rounds (MTR-style)
    #[arg(short = 'c', long, default_value = "10")]
    count: u32,

    /// Maximum number of hops (TTL limit)
    #[arg(short = 'm', long, default_value = "30")]
    max_hops: u8,

    /// Timeout per probe in milliseconds
    #[arg(short = 't', long, default_value = "1000")]
    timeout: u64,

    /// Run in interactive MTR-style live TUI mode (default when no export flags are set)
    #[arg(long)]
    live: bool,

    /// Start the local Web Dashboard server
    #[arg(short = 'w', long)]
    web: bool,

    /// Port for the Web Dashboard server
    #[arg(long, default_value = "8080")]
    web_port: u16,

    /// Export final trace results to a JSON file (prints to stdout if no file path specified)
    #[arg(long, num_args = 0..=1, default_missing_value = "stdout")]
    json: Option<String>,

    /// Export final trace results to a CSV file (prints to stdout if no file path specified)
    #[arg(long, num_args = 0..=1, default_missing_value = "stdout")]
    csv: Option<String>,

    /// Export final trace path as a GeoJSON file (prints to stdout if no file path specified)
    #[arg(long, num_args = 0..=1, default_missing_value = "stdout")]
    geojson: Option<String>,

    /// Print ASCII geographic and ASN maps after tracing
    #[arg(long)]
    map: bool,

    /// Destination port (default: 33434 for UDP, 80 for TCP, 0 for ICMP)
    #[arg(short = 'p', long)]
    port: Option<u16>,

    /// Perform a direct GeoIP lookup on the target address instead of tracing
    #[arg(long)]
    geo: bool,

    /// Force IPv4 resolution
    #[arg(short = '4', long, conflicts_with = "ipv6")]
    ipv4: bool,

    /// Force IPv6 resolution
    #[arg(short = '6', long, conflicts_with = "ipv4")]
    ipv6: bool,

    /// Print a detailed route performance analysis summary at the end
    #[arg(short = 'a', long)]
    analyze: bool,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    // 1. Check if starting the Web Dashboard
    if args.web {
        if let Err(e) = web_dashboard::start_web_server(args.web_port).await {
            eprintln!("Error starting web dashboard: {}", e);
            std::process::exit(1);
        }
        return;
    }

    let target = args
        .target
        .expect("Target host or IP is required when not running web server");

    if args.geo {
        println!("Performing GeoIP lookup for target: {}...", target);
        match dns::resolve_host(&target).await {
            Ok(ips) => {
                if let Some(ip) = ips.first() {
                    println!("Resolved IP: {}", ip);
                    match geoip::lookup_geoip(&ip.to_string()).await {
                        Some(geo) => {
                            println!();
                            println!("IP:       {}", geo.ip);
                            println!("Country:  {}", geo.country);
                            println!("City:     {}", geo.city);
                            println!("ASN:      {}", geo.asn);
                            println!("ISP:      {}", geo.isp);
                            println!("Lat:      {:.4}", geo.lat);
                            println!("Lon:      {:.4}", geo.lon);
                            println!("Timezone: {}", geo.timezone);
                            println!();
                        }
                        None => {
                            eprintln!("Error: GeoIP lookup failed.");
                            std::process::exit(1);
                        }
                    }
                } else {
                    eprintln!(
                        "Error: DNS resolution returned no IP addresses for {}",
                        target
                    );
                    std::process::exit(1);
                }
            }
            Err(e) => {
                eprintln!("Error: DNS resolution failed for {}: {}", target, e);
                std::process::exit(1);
            }
        }
        return;
    }

    // Determine probe method
    let method = if args.tcp {
        ProbeMethod::TCP
    } else if args.udp {
        ProbeMethod::UDP
    } else {
        ProbeMethod::ICMP
    };

    // Determine base port
    let port = args.port.unwrap_or(match method {
        ProbeMethod::UDP => 33434,
        ProbeMethod::TCP => 80,
        ProbeMethod::ICMP => 0,
    });

    let config = TraceConfig {
        target: target.clone(),
        method,
        max_hops: args.max_hops,
        count: args.count,
        timeout: Duration::from_millis(args.timeout),
        port,
        force_ipv4: args.ipv4,
        force_ipv6: args.ipv6,
    };

    // 2. Determine UI or export mode
    let is_export = args.json.is_some() || args.csv.is_some() || args.geojson.is_some() || args.map;

    if is_export && !args.live {
        // ── Non-interactive export / map mode ─────────────────────────────
        println!(
            "Tracing route to {} ({} hops, {} rounds, method: {:?})...",
            target, args.max_hops, args.count, method
        );

        let (tx, mut rx) = tokio::sync::mpsc::channel::<HopStats>(200);
        let cancel_flag = Arc::new(AtomicBool::new(false));

        // Start trace in background
        let config_clone = config.clone();
        let cancel_clone = cancel_flag.clone();
        let runner =
            tokio::spawn(async move { run_traceroute(config_clone, tx, cancel_clone).await });

        // Collect hop stats
        let mut hops: Vec<HopStats> = (1..=args.max_hops).map(HopStats::new).collect();
        while let Some(hop) = rx.recv().await {
            let idx = (hop.hop_num - 1) as usize;
            if idx < hops.len() {
                hops[idx] = hop;
            } else if idx == hops.len() {
                hops.push(hop);
            }
        }

        // Wait for run completion
        match runner.await {
            Ok(Ok(_)) => {
                // Remove trailing hops with zero responses
                while hops.len() > 1 && hops.last().map(|h| h.sent == 0).unwrap_or(false) {
                    hops.pop();
                }

                // Export to requested formats
                if let Some(ref path) = args.json {
                    match export::export_json(path, &hops) {
                        Ok(_) => {
                            if path != "stdout" {
                                println!("✅ Exported JSON  → {}", path);
                            }
                        }
                        Err(e) => eprintln!("❌ JSON export error: {}", e),
                    }
                }

                if let Some(ref path) = args.csv {
                    match export::export_csv(path, &hops) {
                        Ok(_) => {
                            if path != "stdout" {
                                println!("✅ Exported CSV   → {}", path);
                            }
                        }
                        Err(e) => eprintln!("❌ CSV export error: {}", e),
                    }
                }

                if let Some(ref path) = args.geojson {
                    match export::export_geojson(path, &hops) {
                        Ok(_) => {
                            if path != "stdout" {
                                println!("✅ Exported GeoJSON → {}", path);
                            }
                        }
                        Err(e) => eprintln!("❌ GeoJSON export error: {}", e),
                    }
                }

                if args.map {
                    map::print_ascii_map(&hops);
                    map::print_asn_path(&hops);
                    map::print_latency_chart(&hops);
                    map::print_heatmap(&hops);
                }

                // Always print summary table unless exporting to stdout
                let is_stdout_export = args.json.as_deref() == Some("stdout")
                    || args.csv.as_deref() == Some("stdout")
                    || args.geojson.as_deref() == Some("stdout");
                if !is_stdout_export {
                    print_table_summary(&hops, args.analyze);
                }
            }
            Ok(Err(e)) => {
                eprintln!("Traceroute run error: {}", e);
                std::process::exit(1);
            }
            Err(e) => {
                eprintln!("Task panicked: {}", e);
                std::process::exit(1);
            }
        }
    } else {
        // ── Interactive TUI (live MTR mode) ────────────────────────────────
        if let Err(e) = tui::run_tui(config).await {
            eprintln!("TUI Error: {}", e);
            std::process::exit(1);
        }
    }
}

/// Format a simple heatmap bar for terminal output
fn heatmap_bar_simple(rtt_ms: f64, max_rtt: f64) -> String {
    if max_rtt <= 0.0 || rtt_ms <= 0.0 {
        return String::new();
    }
    let ratio = (rtt_ms / max_rtt).min(1.0);
    let filled = ((ratio * 20.0).round() as usize).max(1);
    "█".repeat(filled)
}

fn print_table_summary(hops: &[HopStats], show_analysis: bool) {
    let max_rtt = hops
        .iter()
        .filter_map(|h| if h.recv > 0 { Some(h.avg_rtt()) } else { None })
        .fold(0.0_f64, f64::max);

    println!();
    println!(
        "{:<4} {:<20} {:<25} {:<10} {:<9} {:<6} RTT Heat",
        "Hop", "IP Address", "Country/City", "ASN", "Avg RTT", "Loss%"
    );
    println!("{}", "─".repeat(95));

    for hop in hops {
        if hop.sent == 0 {
            continue;
        }
        let ip_str = hop
            .ip
            .map(|ip| ip.to_string())
            .unwrap_or_else(|| "* * *".to_string());
        let host_str = match &hop.hostname {
            Some(h) if h != &ip_str => h.clone(),
            _ => String::new(),
        };

        let (country_city, asn) = if let Some(ref geo) = hop.geo_info {
            let loc = if geo.city.is_empty() || geo.city == "Unknown" || geo.city == "LAN" {
                geo.country.clone()
            } else {
                format!("{}, {}", geo.country, geo.city)
            };
            (loc, geo.asn.clone())
        } else {
            ("Unknown".to_string(), "N/A".to_string())
        };

        let rtt_str = if hop.recv > 0 {
            crate::util::format_ms(hop.avg_rtt())
        } else {
            "*".to_string()
        };

        let bar = heatmap_bar_simple(hop.avg_rtt(), max_rtt);

        println!(
            "{:<4} {:<20} {:<25} {:<10} {:<9} {:.1}%  {}",
            hop.hop_num,
            &ip_str[..ip_str.len().min(20)],
            &country_city[..country_city.len().min(25)],
            &asn[..asn.len().min(10)],
            rtt_str,
            hop.loss_pct(),
            bar,
        );

        if !host_str.is_empty() {
            println!("{:<26}{}", "", &host_str[..host_str.len().min(50)]);
        }
    }
    println!();

    // Route Performance Analysis (Optional)
    if show_analysis {
        let mut responsive_hops: Vec<&HopStats> =
            hops.iter().filter(|h| h.sent > 0 && h.recv > 0).collect();

        if !responsive_hops.is_empty() {
            println!("╔═══════════════════════════════════════════════════════════════════════════════════════════════════╗");
            println!("║   RouteScope — Route Performance Analysis                                                         ║");
            println!("╚═══════════════════════════════════════════════════════════════════════════════════════════════════╝");

            // 1. Sort by RTT (fastest first)
            responsive_hops.sort_by(|a, b| {
                a.avg_rtt()
                    .partial_cmp(&b.avg_rtt())
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            println!("  ⚡ Fastest Hops (Lowest Latency):");
            for hop in responsive_hops.iter().take(3) {
                let ip_str = hop
                    .ip
                    .map(|ip| ip.to_string())
                    .unwrap_or_else(|| "* * *".to_string());
                println!(
                    "    - Hop {:<2} [Avg RTT: {:>8}]: {}",
                    hop.hop_num,
                    crate::util::format_ms(hop.avg_rtt()),
                    ip_str
                );
            }

            // 2. Sort by RTT (slowest first)
            responsive_hops.reverse();
            println!("\n  ⚠️ Slowest Hops (Highest Latency):");
            for hop in responsive_hops.iter().take(3) {
                let ip_str = hop
                    .ip
                    .map(|ip| ip.to_string())
                    .unwrap_or_else(|| "* * *".to_string());
                println!(
                    "    - Hop {:<2} [Avg RTT: {:>8}]: {}",
                    hop.hop_num,
                    crate::util::format_ms(hop.avg_rtt()),
                    ip_str
                );
            }

            // 3. Reliable hops (0% loss)
            let reliable_hops: Vec<u8> = hops
                .iter()
                .filter(|h| h.sent > 0 && h.loss_pct() == 0.0)
                .map(|h| h.hop_num)
                .collect();
            if !reliable_hops.is_empty() {
                print!("\n  ✅ Reliable Hops (0% Packet Loss): Hops ");
                for (i, hn) in reliable_hops.iter().enumerate() {
                    if i > 0 {
                        print!(", ");
                    }
                    print!("{}", hn);
                }
                println!();
            }

            // 4. Unreliable hops (Loss > 0%)
            let mut unreliable_hops: Vec<&HopStats> = hops
                .iter()
                .filter(|h| h.sent > 0 && h.loss_pct() > 0.0)
                .collect();
            if !unreliable_hops.is_empty() {
                unreliable_hops.sort_by(|a, b| {
                    b.loss_pct()
                        .partial_cmp(&a.loss_pct())
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
                println!("\n  🚨 Unreliable Hops (Highest Packet Loss):");
                for hop in unreliable_hops.iter().take(3) {
                    let ip_str = hop
                        .ip
                        .map(|ip| ip.to_string())
                        .unwrap_or_else(|| "* * *".to_string());
                    println!(
                        "    - Hop {:<2} [Loss: {:>5.1}%]: {}",
                        hop.hop_num,
                        hop.loss_pct(),
                        ip_str
                    );
                }
            }
            println!();
        }
    }
}
