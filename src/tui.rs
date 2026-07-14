use crate::stats::HopStats;
use crate::traceroute::{run_traceroute, TraceConfig};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Gauge, Paragraph, Row, Sparkline, Table, TableState},
    Terminal,
};
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

struct TuiState {
    hops: Vec<HopStats>,
    table_state: TableState,
    paused: bool,
    target: String,
    method_name: String,
    trace_done: bool,
}

pub async fn run_tui(config: TraceConfig) -> Result<(), String> {
    // 1. Setup terminal
    enable_raw_mode().map_err(|e| format!("Failed to enable raw mode: {}", e))?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)
        .map_err(|e| format!("Failed to initialize terminal: {}", e))?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal =
        Terminal::new(backend).map_err(|e| format!("Failed to create terminal: {}", e))?;

    // 2. Initialize application state
    let target = config.target.clone();
    let method_name = format!("{:?}", config.method);
    let (tx, mut rx) = mpsc::channel::<HopStats>(200);
    let cancel_flag = Arc::new(AtomicBool::new(false));

    let mut state = TuiState {
        hops: (1..=config.max_hops).map(HopStats::new).collect(),
        table_state: TableState::default(),
        paused: false,
        target,
        method_name,
        trace_done: false,
    };
    state.table_state.select(Some(0));

    // Spawn traceroute loop
    let cancel_flag_clone = cancel_flag.clone();
    let runner_handle = tokio::spawn(async move {
        let _ = run_traceroute(config, tx, cancel_flag_clone).await;
    });

    // 3. Main event & render loop
    let mut last_tick = Instant::now();
    let tick_rate = Duration::from_millis(100);

    let res = loop {
        // Draw the interface
        if let Err(e) = terminal.draw(|f| draw_ui(f, &mut state)) {
            break Err(format!("Draw error: {}", e));
        }

        // Handle events
        let timeout = tick_rate
            .checked_sub(last_tick.elapsed())
            .unwrap_or_else(|| Duration::from_secs(0));

        if crossterm::event::poll(timeout).unwrap_or(false) {
            if let Ok(Event::Key(key)) = event::read() {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => {
                        break Ok(());
                    }
                    KeyCode::Char(' ') => {
                        state.paused = !state.paused;
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        let i = match state.table_state.selected() {
                            Some(i) => {
                                if i >= state.hops.len().saturating_sub(1) {
                                    0
                                } else {
                                    i + 1
                                }
                            }
                            None => 0,
                        };
                        state.table_state.select(Some(i));
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        let i = match state.table_state.selected() {
                            Some(i) => {
                                if i == 0 {
                                    state.hops.len().saturating_sub(1)
                                } else {
                                    i - 1
                                }
                            }
                            None => 0,
                        };
                        state.table_state.select(Some(i));
                    }
                    _ => {}
                }
            }
        }

        // Process incoming updates from traceroute channel
        while let Ok(hop) = rx.try_recv() {
            if !state.paused {
                let idx = (hop.hop_num - 1) as usize;
                if idx < state.hops.len() {
                    state.hops[idx] = hop;
                } else if idx == state.hops.len() {
                    state.hops.push(hop);
                }
            }
        }

        // Trim trailing hops with no data
        if runner_handle.is_finished() {
            state.trace_done = true;
            while state.hops.len() > 1 && state.hops.last().map(|h| h.sent == 0).unwrap_or(false) {
                state.hops.pop();
            }
        }

        if last_tick.elapsed() >= tick_rate {
            last_tick = Instant::now();
        }
    };

    // 4. Restore terminal
    cancel_flag.store(true, Ordering::Relaxed);
    let _ = runner_handle.await;

    disable_raw_mode().unwrap();
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )
    .unwrap();
    terminal.show_cursor().unwrap();

    res
}

/// Returns a color for latency value in ms
fn rtt_color(rtt_ms: f64) -> Color {
    if rtt_ms < 20.0 {
        Color::Green
    } else if rtt_ms < 80.0 {
        Color::Yellow
    } else if rtt_ms < 200.0 {
        Color::LightRed
    } else {
        Color::Red
    }
}

/// Returns a heatmap bar string (filled blocks proportional to RTT)
fn heatmap_bar(rtt_ms: f64, max_rtt: f64, width: usize) -> String {
    if max_rtt <= 0.0 || rtt_ms <= 0.0 {
        return " ".repeat(width);
    }
    let ratio = (rtt_ms / max_rtt).min(1.0);
    let filled = (ratio * width as f64).round() as usize;
    let filled = filled.max(1).min(width);
    "█".repeat(filled) + &" ".repeat(width - filled)
}

fn draw_ui(f: &mut ratatui::Frame, state: &mut TuiState) {
    let size = f.size();
    let show_heatmap = size.width >= 100;
    let show_geo = size.width >= 130;

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header banner
            Constraint::Min(8),    // Hops Table
            Constraint::Length(9), // Detail Panel / Sparkline
        ])
        .split(size);

    // ─── Header ────────────────────────────────────────────────────────────
    let status_text = if state.trace_done {
        " [DONE]"
    } else if state.paused {
        " [PAUSED]"
    } else {
        ""
    };
    let header_text = format!(
        " 🌐 RouteScope v0.1.0  │  Target: {}  │  Method: {}  │  [↑↓/jk] select  [Space] pause  [q] quit{}",
        state.target, state.method_name, status_text
    );
    let header_style = if state.trace_done {
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    };
    let header = Paragraph::new(header_text).style(header_style).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray)),
    );
    f.render_widget(header, chunks[0]);

    // ─── Calculate global max RTT for heatmap ──────────────────────────────
    let max_rtt = state
        .hops
        .iter()
        .filter_map(|h| if h.recv > 0 { Some(h.avg_rtt()) } else { None })
        .fold(0.0_f64, f64::max);

    // ─── Build column definitions based on terminal width ──────────────────
    let mut col_names: Vec<&str> = vec![
        "Hop",
        "Host / IP",
        "Hostname",
        "Loss%",
        "Snt",
        "Rcv",
        "Last",
        "Avg",
        "Best",
        "Wrst",
        "Jitter",
    ];
    let mut col_constraints: Vec<Constraint> = vec![
        Constraint::Length(5),  // Hop
        Constraint::Length(17), // IP
        Constraint::Min(18),    // Hostname
        Constraint::Length(7),  // Loss%
        Constraint::Length(5),  // Snt
        Constraint::Length(5),  // Rcv
        Constraint::Length(9),  // Last
        Constraint::Length(9),  // Avg
        Constraint::Length(9),  // Best
        Constraint::Length(9),  // Wrst
        Constraint::Length(8),  // Jitter
    ];

    if show_geo {
        col_names.push("Country");
        col_names.push("City");
        col_names.push("ASN");
        col_constraints.push(Constraint::Length(14));
        col_constraints.push(Constraint::Length(14));
        col_constraints.push(Constraint::Length(10));
    }

    if show_heatmap {
        col_names.push("RTT Heat");
        col_constraints.push(Constraint::Length(18));
    }

    let selected_style = Style::default()
        .add_modifier(Modifier::REVERSED)
        .fg(Color::Cyan);
    let header_cells = col_names.iter().map(|h| {
        Cell::from(*h).style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
    });
    let table_header = Row::new(header_cells).height(1);

    let rows: Vec<Row> = state
        .hops
        .iter()
        .map(|hop| {
            let hop_num_str = hop.hop_num.to_string();

            let ip_str = hop
                .ip
                .map(|ip| ip.to_string())
                .unwrap_or_else(|| "* * *".to_string());
            let hostname_str = match &hop.hostname {
                Some(h) if *h != ip_str => h.clone(),
                _ => String::new(),
            };

            // Loss
            let loss_pct = hop.loss_pct();
            let loss_str = format!("{:.1}%", loss_pct);
            let loss_color = if loss_pct > 20.0 {
                Color::Red
            } else if loss_pct > 0.0 {
                Color::LightRed
            } else {
                Color::Green
            };

            // RTT strings
            let last_rtt_str = hop
                .last_rtt
                .map(|r| format!("{:.1}", r))
                .unwrap_or_else(|| "*".to_string());
            let avg_rtt = hop.avg_rtt();
            let avg_rtt_str = if hop.recv > 0 {
                format!("{:.1}", avg_rtt)
            } else {
                "*".to_string()
            };
            let min_rtt_str = if hop.recv > 0 {
                format!("{:.1}", hop.min_rtt())
            } else {
                "*".to_string()
            };
            let max_rtt_str = if hop.recv > 0 {
                format!("{:.1}", hop.max_rtt())
            } else {
                "*".to_string()
            };
            let jitter_str = if hop.recv > 1 {
                format!("{:.1}", hop.jitter())
            } else {
                "*".to_string()
            };

            let avg_color = if hop.recv > 0 {
                rtt_color(avg_rtt)
            } else {
                Color::DarkGray
            };

            let mut cells = vec![
                Cell::from(hop_num_str).style(
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Cell::from(ip_str).style(
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
                Cell::from(hostname_str).style(Style::default().fg(Color::DarkGray)),
                Cell::from(loss_str)
                    .style(Style::default().fg(loss_color).add_modifier(Modifier::BOLD)),
                Cell::from(hop.sent.to_string()).style(Style::default().fg(Color::DarkGray)),
                Cell::from(hop.recv.to_string()).style(Style::default().fg(Color::DarkGray)),
                Cell::from(last_rtt_str),
                Cell::from(avg_rtt_str)
                    .style(Style::default().fg(avg_color).add_modifier(Modifier::BOLD)),
                Cell::from(min_rtt_str).style(Style::default().fg(Color::Green)),
                Cell::from(max_rtt_str).style(Style::default().fg(Color::LightRed)),
                Cell::from(jitter_str).style(Style::default().fg(Color::LightMagenta)),
            ];

            if show_geo {
                let (country, city, asn) = if let Some(ref geo) = hop.geo_info {
                    let c = if geo.country == "Local Network" {
                        "Local"
                    } else {
                        &geo.country
                    };
                    (c.to_string(), geo.city.clone(), geo.asn.clone())
                } else {
                    ("─".to_string(), "─".to_string(), "─".to_string())
                };
                cells.push(Cell::from(country).style(Style::default().fg(Color::LightBlue)));
                cells.push(Cell::from(city).style(Style::default().fg(Color::LightBlue)));
                cells.push(Cell::from(asn).style(Style::default().fg(Color::Magenta)));
            }

            if show_heatmap {
                let bar = heatmap_bar(avg_rtt, max_rtt, 16);
                cells.push(Cell::from(bar).style(Style::default().fg(avg_color)));
            }

            Row::new(cells).height(1)
        })
        .collect();

    let table = Table::new(rows, col_constraints.as_slice())
        .header(table_header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(
                    " 📡 Live Hop Metrics — {} hops{} ",
                    state.hops.len(),
                    if state.trace_done {
                        " (trace complete)"
                    } else {
                        ""
                    }
                ))
                .border_style(Style::default().fg(Color::DarkGray)),
        )
        .highlight_style(selected_style)
        .highlight_symbol("▶ ");

    f.render_stateful_widget(table, chunks[1], &mut state.table_state);

    // ─── Detail Panel ───────────────────────────────────────────────────────
    draw_detail_panel(f, state, chunks[2], max_rtt, show_heatmap);
}

fn draw_detail_panel(
    f: &mut ratatui::Frame,
    state: &mut TuiState,
    area: Rect,
    _max_rtt: f64,
    show_heatmap: bool,
) {
    let selected_idx = state.table_state.selected().unwrap_or(0);
    if selected_idx >= state.hops.len() {
        return;
    }
    let hop = &state.hops[selected_idx];

    let ip_title = hop
        .ip
        .map(|i| i.to_string())
        .unwrap_or_else(|| "* * *".to_string());

    let host_str = match &hop.hostname {
        Some(h) if h != &ip_title => h.clone(),
        _ => "N/A".to_string(),
    };

    // Split into 3 panels: GeoIP | Sparkline | Loss gauge
    let panel_constraints = if show_heatmap {
        vec![
            Constraint::Percentage(35),
            Constraint::Percentage(45),
            Constraint::Percentage(20),
        ]
    } else {
        vec![Constraint::Percentage(45), Constraint::Percentage(55)]
    };

    let detail_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(panel_constraints)
        .split(area);

    // ── GeoIP / Metadata Panel ──
    let geo_lines = if let Some(ref geo) = hop.geo_info {
        let flag = if geo.country == "Local Network" {
            "🏠"
        } else {
            "🌍"
        };
        vec![
            Line::from(vec![
                Span::styled("  Country:  ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("{} {}", flag, geo.country),
                    Style::default()
                        .fg(Color::LightBlue)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(vec![
                Span::styled("  City:     ", Style::default().fg(Color::DarkGray)),
                Span::styled(geo.city.clone(), Style::default().fg(Color::LightBlue)),
            ]),
            Line::from(vec![
                Span::styled("  Hostname: ", Style::default().fg(Color::DarkGray)),
                Span::styled(host_str, Style::default().fg(Color::LightBlue)),
            ]),
            Line::from(vec![
                Span::styled("  ASN:      ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    geo.asn.clone(),
                    Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(vec![
                Span::styled("  ISP:      ", Style::default().fg(Color::DarkGray)),
                Span::styled(geo.isp.clone(), Style::default().fg(Color::White)),
            ]),
            Line::from(vec![
                Span::styled("  Lat/Lon:  ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("{:.4}, {:.4}", geo.lat, geo.lon),
                    Style::default().fg(Color::Yellow),
                ),
            ]),
            Line::from(vec![
                Span::styled("  Timezone: ", Style::default().fg(Color::DarkGray)),
                Span::styled(geo.timezone.clone(), Style::default().fg(Color::White)),
            ]),
        ]
    } else if hop.ip.is_none() {
        vec![Line::from(Span::styled(
            "  No response from this hop  ",
            Style::default().fg(Color::DarkGray),
        ))]
    } else {
        vec![Line::from(Span::styled(
            "  Resolving GeoIP…  ",
            Style::default().fg(Color::Yellow),
        ))]
    };

    let geo_panel = Paragraph::new(geo_lines).block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" 🗺  Hop {} — {} ", hop.hop_num, ip_title))
            .border_style(Style::default().fg(Color::DarkGray)),
    );
    f.render_widget(geo_panel, detail_chunks[0]);

    // ── Sparkline Panel ──
    let spark_data: Vec<u64> = hop.rtts.iter().map(|r| (*r * 10.0) as u64).collect();

    let sparkline = Sparkline::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" 📈 RTT History (×0.1ms) ")
                .border_style(Style::default().fg(Color::DarkGray)),
        )
        .data(&spark_data)
        .style(Style::default().fg(Color::LightGreen));
    f.render_widget(sparkline, detail_chunks[1]);

    // ── Loss Gauge Panel (only if wide enough) ──
    if show_heatmap && detail_chunks.len() > 2 {
        let loss_pct = hop.loss_pct();
        let loss_color = if loss_pct > 20.0 {
            Color::Red
        } else if loss_pct > 5.0 {
            Color::Yellow
        } else {
            Color::Green
        };

        let gauge = Gauge::default()
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" 📉 Packet Loss ")
                    .border_style(Style::default().fg(Color::DarkGray)),
            )
            .gauge_style(Style::default().fg(loss_color))
            .percent(loss_pct.min(100.0) as u16)
            .label(format!(
                "{:.1}% loss ({}/{} pkts)",
                loss_pct, hop.recv, hop.sent
            ));
        f.render_widget(gauge, detail_chunks[2]);
    }
}
