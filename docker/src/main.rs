//! Console UI for Docker containers (status, image, command).

use anyhow::{Context, Result};
use bollard::query_parameters::ListContainersOptionsBuilder;
use bollard::{API_DEFAULT_VERSION, Docker};
use clap::Parser;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::ExecutableCommand;
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Modifier, Style, Stylize};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState};
use ratatui::{Frame, Terminal};
use std::io::stdout;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Parser, Debug)]
#[command(name = "st-docker", about = "Docker containers (TUI)")]
struct Cli {
    #[arg(long, global = true, env = "SERVER_TOOLS_CONFIG", value_name = "PATH")]
    config: Option<PathBuf>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let cfg = st_common::load_config(cli.config.as_deref())?;
    st_common::init_tracing(&cfg.global);

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(run_tui(cfg))
}

async fn run_tui(cfg: st_common::ServerToolsConfig) -> Result<()> {
    let poll = Duration::from_millis(cfg.docker.refresh_ms.max(500));
    let docker = connect_docker(&cfg)?;

    let mut table_state = TableState::default().with_selected(0);

    stdout().execute(EnterAlternateScreen)?;
    crossterm::terminal::enable_raw_mode()?;
    let mut terminal = Terminal::new(ratatui::backend::CrosstermBackend::new(stdout()))?;

    loop {
        let rows = list_containers(&docker, &cfg).await.unwrap_or_default();

        terminal.draw(|f| ui(f, &cfg, &rows, &mut table_state))?;

        if event::poll(poll)? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => break,
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
    }

    crossterm::terminal::disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;
    Ok(())
}

fn connect_docker(cfg: &st_common::ServerToolsConfig) -> Result<Docker> {
    let path = cfg.docker.socket_path.trim();
    Docker::connect_with_unix(path, 120, API_DEFAULT_VERSION)
        .with_context(|| format!("connect Docker socket {path}"))
}

async fn list_containers(
    docker: &Docker,
    cfg: &st_common::ServerToolsConfig,
) -> Result<Vec<ContainerRow>> {
    let opt = ListContainersOptionsBuilder::default()
        .all(cfg.docker.list_all_containers)
        .build();
    let list = docker.list_containers(Some(opt)).await?;
    let mut rows = Vec::new();
    for c in list {
        let id = c
            .id
            .as_deref()
            .map(|s| &s[..s.len().min(12)])
            .unwrap_or("—")
            .to_string();
        let image = c.image.unwrap_or_else(|| "—".into());
        let status = c.status.unwrap_or_else(|| "—".into());
        let names = c
            .names
            .as_ref()
            .map(|n| n.join(", "))
            .unwrap_or_else(|| "—".into());
        rows.push(ContainerRow {
            id,
            names,
            image,
            status,
        });
    }
    Ok(rows)
}

struct ContainerRow {
    id: String,
    names: String,
    image: String,
    status: String,
}

fn ui(
    f: &mut Frame<'_>,
    cfg: &st_common::ServerToolsConfig,
    rows: &[ContainerRow],
    table_state: &mut TableState,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(6),
            Constraint::Length(2),
        ])
        .split(f.area());

    let title = Line::from(vec![
        " st-docker ".bold(),
        "│".dark_gray(),
        " containers ".into(),
        "│".dark_gray(),
        format!(" {} ", cfg.docker.socket_path).dark_gray(),
        "│".dark_gray(),
        " q quit ".dark_gray(),
    ]);
    f.render_widget(
        Paragraph::new(title).block(Block::default().borders(Borders::ALL).title("Docker")),
        chunks[0],
    );

    let table_rows: Vec<Row> = rows
        .iter()
        .map(|r| {
            Row::new(vec![
                Cell::from(r.id.as_str()),
                Cell::from(r.names.as_str()),
                Cell::from(r.image.as_str()),
                Cell::from(r.status.as_str()),
            ])
        })
        .collect();

    let table = Table::new(
        table_rows,
        [
            Constraint::Length(14),
            Constraint::Percentage(35),
            Constraint::Percentage(25),
            Constraint::Percentage(26),
        ],
    )
    .header(
        Row::new(vec!["Id", "Names", "Image", "Status"])
            .style(Style::default().add_modifier(Modifier::BOLD)),
    )
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title("Containers (↑/↓) — requires permission to the Docker socket"),
    )
    .row_highlight_style(Style::default().bg(ratatui::style::Color::DarkGray));

    f.render_stateful_widget(table, chunks[1], table_state);

    f.render_widget(
        Paragraph::new(" Add user to `docker` group or use rootless socket in [docker].socket_path ")
            .style(Style::default().fg(ratatui::style::Color::DarkGray)),
        chunks[2],
    );
}
