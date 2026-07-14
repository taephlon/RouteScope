use crate::stats::HopStats;

/// Print ASCII geographic path map (country/city chain)
pub fn print_ascii_map(stats: &[HopStats]) {
    println!("\n╔══════════════════════════════════════╗");
    println!("║   RouteScope — Geographic Path       ║");
    println!("╚══════════════════════════════════════╝");

    let mut last_location: Option<String> = None;
    let mut path_elements: Vec<(String, String)> = Vec::new(); // (location, asn)

    for hop in stats {
        if let Some(ref geo) = hop.geo_info {
            let city = if geo.city.is_empty() || geo.city == "Unknown" || geo.city == "LAN" {
                String::new()
            } else {
                format!(", {}", geo.city)
            };
            let location = format!("{}{}", geo.country, city);

            if geo.country != "Unknown" && last_location.as_ref() != Some(&location) {
                path_elements.push((location.clone(), geo.asn.clone()));
                last_location = Some(location);
            }
        }
    }

    if path_elements.is_empty() {
        println!("  (no geographic data available)");
        return;
    }

    for (i, (loc, asn)) in path_elements.iter().enumerate() {
        let asn_part = if asn != "N/A" && asn != "Unknown" && !asn.is_empty() {
            format!("  [{}]", asn)
        } else {
            String::new()
        };
        println!("  📍 {}{}", loc, asn_part);
        if i < path_elements.len() - 1 {
            println!("      │");
            println!("      │");
        }
    }
    println!();
}

/// Print ASN transit path
pub fn print_asn_path(stats: &[HopStats]) {
    println!("╔══════════════════════════════════════╗");
    println!("║   RouteScope — ASN Transit Path      ║");
    println!("╚══════════════════════════════════════╝");

    let mut last_asn: Option<String> = None;
    let mut asn_elements: Vec<(String, String)> = Vec::new(); // (asn, isp)

    for hop in stats {
        if let Some(ref geo) = hop.geo_info {
            if geo.asn != "N/A" && geo.asn != "Unknown" && !geo.asn.is_empty() {
                let asn_str = geo.asn.clone();
                if last_asn.as_ref() != Some(&asn_str) {
                    asn_elements.push((asn_str.clone(), geo.isp.clone()));
                    last_asn = Some(asn_str);
                }
            }
        }
    }

    if asn_elements.is_empty() {
        println!("  (no ASN data available)");
        println!();
        return;
    }

    for (i, (asn, isp)) in asn_elements.iter().enumerate() {
        let isp_part = if !isp.is_empty() && isp != "Unknown" {
            format!(" ({})", isp)
        } else {
            String::new()
        };
        println!("  🔗 {}{}", asn, isp_part);
        if i < asn_elements.len() - 1 {
            println!("      │");
        }
    }
    println!();
}

/// Print an ASCII latency chart showing RTT per hop
#[allow(clippy::needless_range_loop)]
pub fn print_latency_chart(stats: &[HopStats]) {
    let active: Vec<&HopStats> = stats.iter().filter(|h| h.recv > 0).collect();
    if active.is_empty() {
        return;
    }

    println!("╔══════════════════════════════════════╗");
    println!("║   RouteScope — Latency Profile       ║");
    println!("╚══════════════════════════════════════╝");

    let max_rtt = active.iter().map(|h| h.avg_rtt()).fold(0.0_f64, f64::max);
    let chart_height = 10usize;
    let chart_width = active.len();

    // Build a height×width grid
    let mut grid = vec![vec![' '; chart_width]; chart_height];
    for (col, hop) in active.iter().enumerate() {
        let rtt = hop.avg_rtt();
        let row = if max_rtt > 0.0 {
            let ratio = rtt / max_rtt;
            let r = ((ratio * (chart_height - 1) as f64).round() as usize).min(chart_height - 1);
            chart_height - 1 - r
        } else {
            chart_height - 1
        };
        grid[row][col] = '●';
    }

    // Print grid with Y-axis labels
    let precision = if max_rtt < 1.0 {
        2
    } else if max_rtt < 10.0 {
        1
    } else {
        0
    };

    for row in 0..chart_height {
        let rtt_label = max_rtt * (chart_height - 1 - row) as f64 / (chart_height - 1) as f64;
        let label = format!("{:>6.*} ┤ ", precision, rtt_label);
        print!("{}", label);
        for col in 0..chart_width {
            print!("{} ", grid[row][col]);
        }
        println!();
    }

    // X-axis
    print!("       └─");
    for _ in 0..chart_width {
        print!("──");
    }
    println!();
    print!("         ");
    for hop in &active {
        print!("{:<2}", hop.hop_num);
    }
    println!("\n         (hop number)");
    println!();
}

/// Print latency heatmap bars per hop
pub fn print_heatmap(stats: &[HopStats]) {
    let active: Vec<&HopStats> = stats.iter().filter(|h| h.recv > 0).collect();
    if active.is_empty() {
        return;
    }

    println!("╔══════════════════════════════════════╗");
    println!("║   RouteScope — Latency Heatmap       ║");
    println!("╚══════════════════════════════════════╝");

    let max_rtt = active.iter().map(|h| h.avg_rtt()).fold(0.0_f64, f64::max);
    let bar_width = 40usize;

    for hop in &active {
        let rtt = hop.avg_rtt();
        let filled = if max_rtt > 0.0 {
            (((rtt / max_rtt) * bar_width as f64).round() as usize)
                .max(1)
                .min(bar_width)
        } else {
            1
        };
        let bar = "█".repeat(filled) + &"░".repeat(bar_width - filled);
        println!("  Hop {:>2}  {:>8.2} ms  {}", hop.hop_num, rtt, bar);
    }
    println!();
}
