//! # RouteScope Traceroute Orchestrator
//!
//! This module coordinates the MTR-style continuous probing loop. Hops are probed
//! sequentially up to the configured max TTL, and statistics (loss, latency, jitter)
//! are calculated and updated in real-time.

use crate::dns::reverse_dns;
use crate::geoip::lookup_geoip;
use crate::probe::{send_probe, ProbeMethod};
use crate::stats::HopStats;
use std::time::Duration;
use tokio::sync::mpsc::Sender;

#[derive(Debug, Clone)]
pub struct TraceConfig {
    pub target: String,
    pub method: ProbeMethod,
    pub max_hops: u8,
    pub count: u32,
    pub timeout: Duration,
    pub port: u16,
    pub force_ipv4: bool,
    pub force_ipv6: bool,
}

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub async fn run_traceroute(
    config: TraceConfig,
    tx: Sender<HopStats>,
    cancel_flag: Arc<AtomicBool>,
) -> Result<(), String> {
    // 1. Resolve target host to IP Address
    let ips = crate::dns::resolve_host(&config.target).await?;
    if ips.is_empty() {
        return Err(format!("No IP address found for host {}", config.target));
    }

    // Filter resolved IP addresses based on flags
    let filtered_ips: Vec<_> = if config.force_ipv4 {
        ips.iter().filter(|ip| ip.is_ipv4()).copied().collect()
    } else if config.force_ipv6 {
        ips.iter().filter(|ip| ip.is_ipv6()).copied().collect()
    } else {
        ips
    };

    if filtered_ips.is_empty() {
        let proto_str = if config.force_ipv4 { "IPv4" } else { "IPv6" };
        return Err(format!(
            "No {} address found for host {}",
            proto_str, config.target
        ));
    }

    // Determine target IP address
    let dest_ip = if config.force_ipv6 {
        filtered_ips[0]
    } else {
        filtered_ips
            .iter()
            .find(|ip| ip.is_ipv4())
            .or_else(|| filtered_ips.first())
            .copied()
            .unwrap()
    };

    let mut hops: Vec<HopStats> = (1..=config.max_hops).map(HopStats::new).collect();

    let mut actual_max_hops = config.max_hops;

    // Run probes in rounds, similar to MTR
    for _round in 1..=config.count {
        if cancel_flag.load(Ordering::Relaxed) {
            break;
        }

        let current_limit = actual_max_hops;
        for ttl in 1..=current_limit {
            if cancel_flag.load(Ordering::Relaxed) {
                break;
            }

            let hop_idx = (ttl - 1) as usize;
            let port = if config.method == ProbeMethod::UDP {
                config.port + ttl as u16 // Standard UDP traceroute increments destination port
            } else {
                config.port
            };

            // Run probe on blocking pool to not block tokio threads (raw sockets poll/blocking wait)
            let dest_ip_clone = dest_ip;
            let method = config.method;
            let timeout = config.timeout;
            let probe_res = tokio::task::spawn_blocking(move || {
                send_probe(dest_ip_clone, method, ttl, port, timeout)
            })
            .await
            .map_err(|e| format!("Task join error: {}", e))?;

            // If we got a response, process DNS and GeoIP
            let hop_ip = probe_res.ip;
            let rtt_ms = probe_res.rtt.map(|d| d.as_secs_f64() * 1000.0);

            // Register metrics
            hops[hop_idx].register_probe(hop_ip, rtt_ms);

            let mut reached_target = false;
            if let Some(ip) = hop_ip {
                // If it is a new IP for this hop, or dns/geo is not loaded, resolve them
                if hops[hop_idx].hostname.is_none() {
                    let hostname = reverse_dns(ip).await;
                    hops[hop_idx].hostname = Some(hostname);
                }

                if hops[hop_idx].geo_info.is_none() {
                    let geo = lookup_geoip(&ip.to_string()).await;
                    hops[hop_idx].geo_info = geo;
                }

                // If reached target, we can shrink actual_max_hops for future rounds
                if probe_res.reached || ip == dest_ip {
                    if ttl < actual_max_hops {
                        actual_max_hops = ttl;
                        hops.truncate(actual_max_hops as usize);
                    }
                    reached_target = true;
                }
            }

            // Send hop update
            let _ = tx.send(hops[hop_idx].clone()).await;

            if reached_target {
                break;
            }

            // Space out probes
            tokio::time::sleep(Duration::from_millis(40)).await;
        }

        // Space out rounds
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    Ok(())
}
