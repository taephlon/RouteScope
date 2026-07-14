use std::net::IpAddr;

pub async fn reverse_dns(ip: IpAddr) -> String {
    tokio::task::spawn_blocking(move || {
        dns_lookup::lookup_addr(&ip).unwrap_or_else(|_| ip.to_string())
    })
    .await
    .unwrap_or_else(|_| ip.to_string())
}

pub async fn resolve_host(host: &str) -> Result<Vec<IpAddr>, String> {
    let host = host.to_string();
    tokio::task::spawn_blocking(move || {
        dns_lookup::lookup_host(&host)
            .map_err(|e| format!("DNS resolution failed for {}: {}", host, e))
    })
    .await
    .map_err(|e| format!("Spawn blocking error: {}", e))?
}
