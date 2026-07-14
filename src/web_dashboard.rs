use crate::probe::ProbeMethod;
use crate::stats::HopStats;
use crate::traceroute::{run_traceroute, TraceConfig};
use axum::{
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    response::Html,
    routing::get,
    Router,
};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

#[derive(Deserialize)]
struct TraceRequest {
    host: String,
    method: String,
    max_hops: Option<u8>,
    count: Option<u32>,
    timeout_ms: Option<u64>,
}

#[derive(Serialize)]
struct FrontendHopState {
    hop_num: u8,
    ip: Option<String>,
    hostname: Option<String>,
    geo_info: Option<crate::geoip::GeoInfo>,
    sent_count: u32,
    recv_count: u32,
    loss_pct: f64,
    last_rtt_ms: Option<f64>,
    avg_rtt_ms: f64,
    jitter_ms: f64,
}

impl FrontendHopState {
    fn from_stats(stats: &HopStats) -> Self {
        Self {
            hop_num: stats.hop_num,
            ip: stats.ip.map(|ip| ip.to_string()),
            hostname: stats.hostname.clone(),
            geo_info: stats.geo_info.clone(),
            sent_count: stats.sent,
            recv_count: stats.recv,
            loss_pct: stats.loss_pct(),
            last_rtt_ms: stats.last_rtt,
            avg_rtt_ms: stats.avg_rtt(),
            jitter_ms: stats.jitter(),
        }
    }
}

pub async fn start_web_server(port: u16) -> Result<(), String> {
    let app = Router::new()
        .route("/", get(index_handler))
        .route("/api/trace/ws", get(ws_handler));

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    println!("------------------------------------------------------------");
    println!("   RouteScope Web Dashboard is running!");
    println!("   Open http://localhost:{} in your browser", port);
    println!("------------------------------------------------------------");

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .map_err(|e| format!("Failed to bind port {}: {}", port, e))?;

    axum::serve(listener, app)
        .await
        .map_err(|e| format!("Server execution error: {}", e))?;

    Ok(())
}

async fn index_handler() -> Html<&'static str> {
    Html(include_str!("dashboard.html"))
}

async fn ws_handler(ws: WebSocketUpgrade) -> axum::response::Response {
    println!("Websocket: Received upgrade request");
    ws.on_failed_upgrade(|err| {
        println!("Websocket: Upgrade failed: {:?}", err);
    })
    .on_upgrade(handle_socket)
}

async fn handle_socket(mut socket: WebSocket) {
    println!("Websocket: Connection established successfully");
    let mut active_cancel = Arc::new(AtomicBool::new(false));
    let (tx, mut rx) = mpsc::channel::<HopStats>(200);

    loop {
        tokio::select! {
            // 1. Listen for new messages from the WebSocket client
            msg_opt = socket.recv() => {
                match msg_opt {
                    Some(Ok(Message::Text(text))) => {
                        if let Ok(req) = serde_json::from_str::<TraceRequest>(&text) {
                            // Cancel any running traceroute first
                            active_cancel.store(true, Ordering::Relaxed);

                            // Recreate cancellation flag for the new traceroute run
                            active_cancel = Arc::new(AtomicBool::new(false));
                            let cancel_for_run = active_cancel.clone();

                            // Parse parameters
                            let method = match req.method.to_uppercase().as_str() {
                                "ICMP" => ProbeMethod::ICMP,
                                "TCP" => ProbeMethod::TCP,
                                _ => ProbeMethod::UDP,
                            };

                            let max_hops = req.max_hops.unwrap_or(30);
                            let count = req.count.unwrap_or(10);
                            let timeout = Duration::from_millis(req.timeout_ms.unwrap_or(1000));

                            let config = TraceConfig {
                                target: req.host,
                                method,
                                max_hops,
                                count,
                                timeout,
                                port: match method {
                                    ProbeMethod::TCP => 80,
                                    _ => 33434,
                                },
                                force_ipv4: false,
                                force_ipv6: false,
                            };

                            let tx_clone = tx.clone();
                            // Spawn the traceroute run loop in the background
                            tokio::spawn(async move {
                                let result = run_traceroute(config, tx_clone, cancel_for_run).await;
                                if let Err(e) = result {
                                    eprintln!("Trace run failed: {}", e);
                                }
                            });
                        } else if text.contains("\"action\":\"stop\"") || text.contains("\"action\": \"stop\"") {
                            active_cancel.store(true, Ordering::Relaxed);
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        // Client disconnected, stop traceroute and exit socket loop
                        active_cancel.store(true, Ordering::Relaxed);
                        break;
                    }
                    _ => {}
                }
            }

            // 2. Receive progress updates from the running traceroute
            hop_opt = rx.recv() => {
                match hop_opt {
                    Some(hop) => {
                        let frontend_state = FrontendHopState::from_stats(&hop);
                        if let Ok(json_str) = serde_json::to_string(&serde_json::json!({
                            "type": "probe",
                            "data": frontend_state
                        })) {
                            if socket.send(Message::Text(json_str)).await.is_err() {
                                active_cancel.store(true, Ordering::Relaxed);
                                break;
                            }
                        }
                    }
                    None => {
                        // Channel closed — traceroute finished
                        let complete_msg = serde_json::json!({ "type": "complete" }).to_string();
                        let _ = socket.send(Message::Text(complete_msg)).await;
                    }
                }
            }
        }
    }
}
