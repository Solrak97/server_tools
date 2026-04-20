//! Console UI for network interfaces (throughput) and TCP listeners.

use anyhow::Result;
use clap::Parser;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::ExecutableCommand;
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState};
use ratatui::{Frame, Terminal};
use std::collections::HashMap;
use std::io::{self, stdout};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use sysinfo::Networks;

#[derive(Parser, Debug)]
#[command(name = "st-network", about = "Network interfaces and listeners (TUI)")]
struct Cli {
    #[arg(long, global = true, env = "SERVER_TOOLS_CONFIG", value_name = "PATH")]
    config: Option<PathBuf>,
}

struct NetSample {
    rx: u64,
    tx: u64,
    at: Instant,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let cfg_path = cli.config.clone();
    let cfg = st_common::load_config(cfg_path.as_deref())?;
    st_common::init_tracing(&cfg.global);

    run_tui(cfg)
}

fn run_tui(cfg: st_common::ServerToolsConfig) -> Result<()> {
    let poll = Duration::from_millis(cfg.network.refresh_ms.max(200));
    let mut networks = Networks::new_with_refreshed_list();
    let mut last: HashMap<String, NetSample> = HashMap::new();
    let mut table_state = TableState::default().with_selected(0);
    let mut show_help = true;

    stdout().execute(EnterAlternateScreen)?;
    crossterm::terminal::enable_raw_mode()?;
    let mut terminal = Terminal::new(ratatui::backend::CrosstermBackend::new(stdout()))?;

    let mut tick = Instant::now();

    loop {
        networks.refresh(true);

        let mut rows: Vec<(String, u64, u64, f64, f64)> = Vec::new();
        for (iface, data) in &networks {
            if cfg.network.hide_interfaces.iter().any(|h| h == iface) {
                continue;
            }
            let rx = data.received();
            let tx = data.transmitted();
            let (rx_s, tx_s) = if let Some(prev) = last.get(iface) {
                let dt = tick.duration_since(prev.at).as_secs_f64().max(0.001);
                (((rx - prev.rx) as f64 / dt), ((tx - prev.tx) as f64 / dt))
            } else {
                (0.0, 0.0)
            };
            rows.push((iface.clone(), rx, tx, rx_s, tx_s));
            last.insert(
                iface.clone(),
                NetSample {
                    rx,
                    tx,
                    at: tick,
                },
            );
        }
        rows.sort_by(|a, b| a.0.cmp(&b.0));

        let listeners = if cfg.network.show_listeners {
            tcp_listeners_ipv4().unwrap_or_default()
        } else {
            Vec::new()
        };

        terminal.draw(|f| {
            ui(
                f,
                &cfg,
                &rows,
                &listeners,
                &mut table_state,
                show_help,
            )
        })?;

        if event::poll(poll)? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => break,
                    KeyCode::Char('?') | KeyCode::F(1) => show_help = !show_help,
                    KeyCode::Down => {
                        let max = rows.len().saturating_sub(1);
                        let i = table_state.selected().unwrap_or(0).saturating_add(1).min(max);
                        table_state.select(Some(i));
                    }
                    KeyCode::Up => {
                        let i = table_state
                            .selected()
                            .unwrap_or(0)
                            .saturating_sub(1);
                        table_state.select(Some(i));
                    }
                    _ => {}
                },
                Event::Resize(_, _) => {}
                _ => {}
            }
        }

        tick = Instant::now();
    }

    crossterm::terminal::disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;
    Ok(())
}

fn ui(
    f: &mut Frame<'_>,
    cfg: &st_common::ServerToolsConfig,
    rows: &[(String, u64, u64, f64, f64)],
    listeners: &[String],
    table_state: &mut TableState,
    show_help: bool,
) {
    let constraints: Vec<Constraint> = if cfg.network.show_listeners {
        vec![
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(10),
            Constraint::Length(2),
        ]
    } else {
        vec![
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(2),
        ]
    };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(f.area());

    let title = Line::from(vec![
        " st-network ".bold(),
        "│".dark_gray(),
        " interfaces ".into(),
        "│".dark_gray(),
        " q quit ".dark_gray(),
        " ? help ".dark_gray(),
    ]);
    f.render_widget(
        Paragraph::new(title).block(Block::default().borders(Borders::ALL).title("Network")),
        chunks[0],
    );

    let header = Row::new(vec![
        Cell::from("Interface"),
        Cell::from("RX total"),
        Cell::from("TX total"),
        Cell::from("RX/s"),
        Cell::from("TX/s"),
    ])
    .style(Style::default().add_modifier(Modifier::BOLD))
    .height(1);

    let table_rows: Vec<Row> = rows
        .iter()
        .map(|(name, rx, tx, rxs, txs)| {
            Row::new(vec![
                Cell::from(name.as_str()),
                Cell::from(format_bytes(*rx)),
                Cell::from(format_bytes(*tx)),
                Cell::from(format_rate(*rxs)),
                Cell::from(format_rate(*txs)),
            ])
        })
        .collect();

    let widths = [
        Constraint::Percentage(28),
        Constraint::Percentage(18),
        Constraint::Percentage(18),
        Constraint::Percentage(18),
        Constraint::Percentage(18),
    ];

    let table = Table::new(table_rows, widths)
        .header(header)
        .block(Block::default().borders(Borders::ALL).title("Interfaces (↑/↓ select)"))
        .row_highlight_style(Style::default().bg(ratatui::style::Color::DarkGray));

    f.render_stateful_widget(table, chunks[1], table_state);

    let footer_idx = if cfg.network.show_listeners { 3 } else { 2 };
    if cfg.network.show_listeners {
        let lis_text: Vec<Line> = if listeners.is_empty() {
            vec![Line::from("No IPv4 listeners parsed (or /proc unavailable).")]
        } else {
            listeners
                .iter()
                .take(chunks[2].height.saturating_sub(2) as usize)
                .map(|s| Line::from(Span::raw(s)))
                .collect()
        };
        f.render_widget(
            Paragraph::new(lis_text).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("TCP listeners (IPv4, /proc/net/tcp)"),
            ),
            chunks[2],
        );
    }

    let footer = if show_help {
        Line::from(
            " ?/F1 toggle this bar │ ↑/↓ move │ q/Esc quit │ config: /etc or XDG server_tools/config.toml ",
        )
    } else {
        Line::from(" Press ? for help ")
    };
    f.render_widget(
        Paragraph::new(footer).style(Style::default().fg(ratatui::style::Color::DarkGray)),
        chunks[footer_idx],
    );
}

fn format_bytes(n: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    if n >= GB {
        format!("{:.2} GiB", n as f64 / GB as f64)
    } else if n >= MB {
        format!("{:.2} MiB", n as f64 / MB as f64)
    } else if n >= KB {
        format!("{:.2} KiB", n as f64 / KB as f64)
    } else {
        format!("{n} B")
    }
}

fn format_rate(bytes_per_s: f64) -> String {
    format!("{}/s", format_bytes(bytes_per_s as u64))
}

/// Parse `/proc/net/tcp` for LISTEN sockets (state 0x0A).
fn tcp_listeners_ipv4() -> io::Result<Vec<String>> {
    let raw = std::fs::read_to_string("/proc/net/tcp")?;
    let mut out = Vec::new();
    for line in raw.lines().skip(1) {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 4 {
            continue;
        }
        if cols[3] != "0A" {
            continue;
        }
        if let Some((ip, port)) = parse_local_addr_v4(cols[1]) {
            out.push(format!("{ip}:{port}"));
        }
    }
    out.sort();
    out.dedup();
    Ok(out)
}

fn parse_local_addr_v4(field: &str) -> Option<(String, u16)> {
    let mut parts = field.split(':');
    let ip_hex = parts.next()?;
    let port_hex = parts.next()?;
    if ip_hex.len() != 8 {
        return None;
    }
    let ip_u = u32::from_str_radix(ip_hex, 16).ok()?;
    let b = ip_u.to_le_bytes();
    let ip = format!("{}.{}.{}.{}", b[0], b[1], b[2], b[3]);
    let port = u16::from_str_radix(port_hex, 16).ok()?;
    Some((ip, port))
}
