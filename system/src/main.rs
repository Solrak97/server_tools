//! Console UI for CPU, memory, swap, disks, and top processes.

use anyhow::Result;
use clap::Parser;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::ExecutableCommand;
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Modifier, Style, Stylize};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Cell, Gauge, Paragraph, Row, Table};
use ratatui::{Frame, Terminal};
use std::io::stdout;
use std::path::PathBuf;
use std::time::Duration;
use sysinfo::{
    CpuRefreshKind, Disks, MemoryRefreshKind, ProcessRefreshKind, ProcessesToUpdate, RefreshKind,
    System,
};

#[derive(Parser, Debug)]
#[command(name = "st-system", about = "CPU, memory, disks, and processes (TUI)")]
struct Cli {
    #[arg(long, global = true, env = "SERVER_TOOLS_CONFIG", value_name = "PATH")]
    config: Option<PathBuf>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let cfg = st_common::load_config(cli.config.as_deref())?;
    st_common::init_tracing(&cfg.global);
    run_tui(cfg)
}

fn run_tui(cfg: st_common::ServerToolsConfig) -> Result<()> {
    let poll = Duration::from_millis(cfg.system.refresh_ms.max(300));

    let mut sys = System::new_with_specifics(
        RefreshKind::nothing()
            .with_cpu(CpuRefreshKind::everything())
            .with_memory(MemoryRefreshKind::everything())
            .with_processes(ProcessRefreshKind::everything()),
    );
    let mut disks = Disks::new_with_refreshed_list();

    stdout().execute(EnterAlternateScreen)?;
    crossterm::terminal::enable_raw_mode()?;
    let mut terminal = Terminal::new(ratatui::backend::CrosstermBackend::new(stdout()))?;

    loop {
        sys.refresh_cpu_all();
        sys.refresh_memory();
        sys.refresh_processes(ProcessesToUpdate::All, true);
        disks.refresh(true);

        terminal.draw(|f| ui(f, &cfg, &sys, &disks))?;

        if event::poll(poll)? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => break,
                    _ => {}
                },
                Event::Resize(_, _) => {}
                _ => {}
            }
        }
    }

    crossterm::terminal::disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;
    Ok(())
}

fn ui(
    f: &mut Frame<'_>,
    cfg: &st_common::ServerToolsConfig,
    sys: &System,
    disks: &Disks,
) {
    let cfg = &cfg.system;
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(if cfg.show_per_cpu {
                2 + sys.cpus().len().clamp(1, 16) as u16
            } else {
                3
            }),
            Constraint::Length(4),
            Constraint::Length(8),
            Constraint::Min(6),
            Constraint::Length(2),
        ])
        .split(f.area());

    let title = Line::from(vec![
        " st-system ".bold(),
        "│".dark_gray(),
        " resources ".into(),
        "│".dark_gray(),
        " q quit ".dark_gray(),
    ]);
    f.render_widget(
        Paragraph::new(title).block(Block::default().borders(Borders::ALL).title("System")),
        chunks[0],
    );

    // CPU gauges
    let cpu_block = Block::default()
        .borders(Borders::ALL)
        .title("CPU %");
    let cpu_inner = cpu_block.inner(chunks[1]);
    f.render_widget(cpu_block, chunks[1]);

    if cfg.show_per_cpu {
        let n = sys.cpus().len().clamp(1, 16);
        let cols = Layout::default()
            .direction(Direction::Vertical)
            .constraints((0..n).map(|_| Constraint::Length(1)).collect::<Vec<_>>())
            .split(cpu_inner);
        for (i, c) in sys.cpus().iter().take(n).enumerate() {
            let g = Gauge::default()
                .label(format!("cpu{i}"))
                .ratio((c.cpu_usage() as f64 / 100.0).min(1.0));
            f.render_widget(g, cols[i]);
        }
    } else {
        let g = Gauge::default()
            .label("global")
            .ratio((sys.global_cpu_usage() as f64 / 100.0).min(1.0));
        f.render_widget(g, cpu_inner);
    }

    // Memory
    let total = sys.total_memory();
    let used = sys.used_memory();
    let mem_ratio = if total > 0 {
        (used as f64 / total as f64).min(1.0)
    } else {
        0.0
    };
    let mem_line = format!(
        "RAM {:.1}% — {} / {} (used/total)",
        mem_ratio * 100.0,
        format_bytes(used),
        format_bytes(total)
    );
    let swap_total = sys.total_swap();
    let swap_used = sys.used_swap();
    let swap_ratio = if swap_total > 0 {
        (swap_used as f64 / swap_total as f64).min(1.0)
    } else {
        0.0
    };
    let swap_g = Gauge::default()
        .label(format!(
            "Swap {:.1}% — {} / {}",
            swap_ratio * 100.0,
            format_bytes(swap_used),
            format_bytes(swap_total)
        ))
        .ratio(swap_ratio);

    let mem_block = Block::default()
        .borders(Borders::ALL)
        .title("Memory / swap");
    let mem_inner = mem_block.inner(chunks[2]);
    f.render_widget(mem_block, chunks[2]);
    let mem_split = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(2)])
        .split(mem_inner);
    f.render_widget(Paragraph::new(mem_line), mem_split[0]);
    f.render_widget(swap_g, mem_split[1]);

    // Disks
    let disk_rows: Vec<Row> = disks
        .iter()
        .map(|d| {
            let total = d.total_space();
            let avail = d.available_space();
            let used = total.saturating_sub(avail);
            let pct = if total > 0 {
                (used as f64 / total as f64 * 100.0) as u8
            } else {
                0
            };
            Row::new(vec![
                Cell::from(d.mount_point().to_string_lossy().to_string()),
                Cell::from(format!("{pct}%")),
                Cell::from(format_bytes(used)),
                Cell::from(format_bytes(total)),
            ])
        })
        .collect();

    let disk_table = Table::new(
        disk_rows,
        [
            Constraint::Percentage(45),
            Constraint::Percentage(15),
            Constraint::Percentage(20),
            Constraint::Percentage(20),
        ],
    )
    .header(
        Row::new(vec!["Mount", "Use%", "Used", "Size"])
            .style(Style::default().add_modifier(Modifier::BOLD)),
    )
    .block(Block::default().borders(Borders::ALL).title("Disks"));
    f.render_widget(disk_table, chunks[3]);

    // Processes
    let mut procs: Vec<_> = sys.processes().values().collect();
    procs.sort_by(|a, b| {
        b.cpu_usage()
            .partial_cmp(&a.cpu_usage())
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let rows: Vec<Row> = procs
        .into_iter()
        .take(cfg.process_limit)
        .map(|p| {
            Row::new(vec![
                Cell::from(format!("{}", p.pid().as_u32())),
                Cell::from(p.name().to_string_lossy().to_string()),
                Cell::from(format!("{:.1}%", p.cpu_usage())),
                Cell::from(format_bytes(p.memory())),
            ])
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(8),
            Constraint::Percentage(50),
            Constraint::Length(8),
            Constraint::Length(12),
        ],
    )
    .header(
        Row::new(vec!["PID", "Name", "CPU%", "RAM"])
            .style(Style::default().add_modifier(Modifier::BOLD)),
    )
    .block(Block::default().borders(Borders::ALL).title("Processes (by CPU)"));
    f.render_widget(table, chunks[4]);

    f.render_widget(
        Paragraph::new(" config: /etc or XDG server_tools/config.toml — refresh_ms in [system] ")
            .style(Style::default().fg(ratatui::style::Color::DarkGray)),
        chunks[5],
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
