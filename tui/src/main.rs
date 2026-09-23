//! Rextto TUI — a small terminal client for a running Rextto daemon.
//!
//! It talks to the daemon's HTTP API (default `http://127.0.0.1:5000`) and shows
//! status, torrents, activity, logs and health, with a few actions (run cycle,
//! pause/resume the selected torrent). It never touches the databases directly.

use std::{
    io::{self, Stdout},
    time::{Duration, Instant},
};

use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, Tabs, Wrap},
    Frame, Terminal,
};
use serde_json::Value;

const REFRESH: Duration = Duration::from_secs(2);
const TABS: [&str; 5] = ["Status", "Torrents", "Activity", "Logs", "Health"];

struct Client {
    base: String,
    token: Option<String>,
}

impl Client {
    fn new(base: String, token: Option<String>) -> Self {
        Self {
            base: base.trim_end_matches('/').to_string(),
            token,
        }
    }

    fn get(&self, path: &str) -> Result<Value, String> {
        let mut request = ureq::get(&format!("{}{}", self.base, path))
            .timeout(Duration::from_secs(15));
        if let Some(token) = &self.token {
            request = request.set("x-rextto-token", token);
        }
        request
            .call()
            .map_err(|error| error.to_string())?
            .into_json::<Value>()
            .map_err(|error| error.to_string())
    }

    fn post(&self, path: &str) -> Result<Value, String> {
        let mut request = ureq::post(&format!("{}{}", self.base, path))
            .timeout(Duration::from_secs(15))
            .set("Content-Type", "application/json");
        if let Some(token) = &self.token {
            request = request.set("x-rextto-token", token);
        }
        request
            .send_json(serde_json::json!({}))
            .map_err(|error| error.to_string())?
            .into_json::<Value>()
            .map_err(|error| error.to_string())
    }
}

struct App {
    client: Client,
    tab: usize,
    status: Result<Value, String>,
    torrents: Result<Value, String>,
    events: Result<Value, String>,
    logs: Result<Value, String>,
    health: Result<Value, String>,
    selected: usize,
    last_refresh: Instant,
    last_logs: Instant,
    message: String,
}

impl App {
    fn new(client: Client) -> Self {
        let mut app = Self {
            client,
            tab: 0,
            status: Err("loading…".into()),
            torrents: Err("loading…".into()),
            events: Err("loading…".into()),
            logs: Err("loading…".into()),
            health: Err("loading…".into()),
            selected: 0,
            last_refresh: Instant::now() - REFRESH,
            last_logs: Instant::now() - REFRESH,
            message: String::new(),
        };
        app.refresh();
        app
    }

    fn refresh(&mut self) {
        self.status = self.client.get("/api/status");
        self.torrents = self.client.get("/api/torrents");
        self.events = self.client.get("/api/torrent-events");
        self.health = self.client.get("/api/health");
        self.refresh_logs();
        let count = self.torrent_list().len();
        if count == 0 {
            self.selected = 0;
        } else if self.selected >= count {
            self.selected = count - 1;
        }
    }

    fn refresh_logs(&mut self) {
        self.logs = self.client.get("/api/logs?limit=200");
        self.last_logs = Instant::now();
    }

    fn torrent_list(&self) -> Vec<Value> {
        self.torrents
            .as_ref()
            .ok()
            .and_then(|value| value.as_array().cloned())
            .unwrap_or_default()
    }

    fn run_cycle(&mut self) {
        self.message = match self.client.post("/api/run-now") {
            Ok(_) => "cycle requested".into(),
            Err(error) => format!("cycle failed: {error}"),
        };
    }

    fn toggle_selected(&mut self) {
        let torrents = self.torrent_list();
        let Some(torrent) = torrents.get(self.selected) else {
            self.message = "no torrent selected".into();
            return;
        };
        let hash = torrent
            .get("hash")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if hash.is_empty() {
            return;
        }
        let paused = torrent
            .get("state")
            .and_then(Value::as_str)
            .is_some_and(|state| state == "paused");
        let action = if paused { "resume" } else { "pause" };
        self.message = match self.client.post(&format!("/api/torrents/{hash}/{action}")) {
            Ok(_) => format!("{action} {hash}"),
            Err(error) => format!("{action} failed: {error}"),
        };
    }
}

fn human_bytes(bytes: f64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes.max(0.0);
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{value:.0} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .map(|item| match item {
            Value::String(text) => text.clone(),
            other => other.to_string(),
        })
        .unwrap_or_default()
}

fn section(title: &str) -> Block<'static> {
    Block::default().borders(Borders::ALL).title(title.to_string())
}

fn draw_status(frame: &mut Frame, area: ratatui::layout::Rect, app: &App) {
    let block = section("Overview");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    match &app.status {
        Ok(status) => {
            let torrents = status.get("torrent_stats").cloned().unwrap_or(Value::Null);
            let cycle = status.get("last_cycle").cloned().unwrap_or(Value::Null);
            let lines = vec![
                Line::from(vec![
                    ratatui::text::Span::styled(
                        format!(" {} ", text(status, "name")),
                        Style::default().add_modifier(Modifier::BOLD),
                    ),
                    ratatui::text::Span::raw(format!("v{}  ", text(status, "version"))),
                    ratatui::text::Span::styled(
                        if status.get("active").and_then(Value::as_bool).unwrap_or(false) {
                            "ACTIVE"
                        } else {
                            "PAUSED"
                        },
                        Style::default().fg(if status.get("active").and_then(Value::as_bool).unwrap_or(false) {
                            Color::Green
                        } else {
                            Color::Yellow
                        }),
                    ),
                    ratatui::text::Span::raw(if status.get("dry_run").and_then(Value::as_bool).unwrap_or(false) {
                        "  dry-run"
                    } else {
                        ""
                    }),
                ]),
                Line::from(format!(
                    "Torrents: {} ({} downloading, {} queued, {} seeding)",
                    text(&torrents, "count"),
                    text(&torrents, "downloading"),
                    text(&torrents, "queued"),
                    text(&torrents, "seeding"),
                )),
                Line::from(format!(
                    "Last cycle: scraped {} | candidates {} | downloads {} | errors {}",
                    text(&cycle, "scraped"),
                    text(&cycle, "candidates"),
                    text(&cycle, "downloads_started"),
                    text(&cycle, "errors"),
                )),
                Line::from(""),
                Line::from("Discover in feeds:"),
                {
                    let seen = status.get("seen").cloned().unwrap_or(Value::Null);
                    Line::from(format!(
                        "  groups {} · movies {} · series {}",
                        text(&seen, "groups"),
                        text(&seen, "movies"),
                        text(&seen, "series"),
                    ))
                },
            ];
            frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
        }
        Err(error) => {
            frame.render_widget(
                Paragraph::new(format!("cannot reach daemon: {error}\n\nexport REXTTO_URL / REXTTO_API_TOKEN"))
                    .style(Style::default().fg(Color::Red))
                    .wrap(Wrap { trim: false }),
                inner,
            );
        }
    }
}

fn draw_torrents(frame: &mut Frame, area: ratatui::layout::Rect, app: &App) {
    let block = section("Torrents  (p = pause/resume)");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let torrents = app.torrent_list();
    let rows = torrents.iter().enumerate().map(|(index, torrent)| {
        let progress = torrent.get("progress").and_then(Value::as_f64).unwrap_or(0.0);
        let down = torrent.get("download_rate").and_then(Value::as_u64).unwrap_or(0);
        let up = torrent.get("upload_rate").and_then(Value::as_u64).unwrap_or(0);
        let done = torrent.get("total_done").and_then(Value::as_i64).unwrap_or(0) as f64;
        let total = torrent.get("total_size").and_then(Value::as_i64).unwrap_or(0) as f64;
        let name = text(torrent, "name");
        let name = if name.len() > 60 {
            format!("{}…", &name[..59])
        } else {
            name
        };
        let row = Row::new(vec![
            Cell::from(name),
            Cell::from(text(torrent, "state")),
            Cell::from(format!("{progress:.1}%")),
            Cell::from(format!("{} / {}", human_bytes(done), human_bytes(total))),
            Cell::from(format!("↓{}/s ↑{}/s", human_bytes(down as f64), human_bytes(up as f64))),
        ]);
        if index == app.selected {
            row.style(Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD))
        } else {
            row
        }
    });
    let table = Table::new(
        rows,
        [
            Constraint::Min(30),
            Constraint::Length(14),
            Constraint::Length(7),
            Constraint::Length(20),
            Constraint::Length(22),
        ],
    )
    .header(
        Row::new(vec!["Name", "State", "Prog", "Size", "Rates"])
            .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
    );
    frame.render_widget(table, inner);
}

fn draw_events(frame: &mut Frame, area: ratatui::layout::Rect, app: &App) {
    let block = section("Activity");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    match &app.events {
        Ok(events) => {
            let lines = events
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .rev()
                        .take(200)
                        .map(|item| {
                            Line::from(format!(
                                "{}  {}  {}",
                                text(item, "kind"),
                                text(item, "hash"),
                                text(item, "name")
                            ))
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
        }
        Err(error) => frame.render_widget(Paragraph::new(error.clone()), inner),
    }
}

fn draw_logs(frame: &mut Frame, area: ratatui::layout::Rect, app: &App) {
    let block = section("Logs");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    match &app.logs {
        Ok(value) => {
            let lines = value
                .get("items")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(Value::as_str)
                        .map(|line| Line::from(line.to_string()))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
        }
        Err(error) => frame.render_widget(Paragraph::new(error.clone()), inner),
    }
}

fn draw_health(frame: &mut Frame, area: ratatui::layout::Rect, app: &App) {
    let block = section("Health");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    match &app.health {
        Ok(value) => {
            let body = serde_json::to_string_pretty(value).unwrap_or_default();
            frame.render_widget(Paragraph::new(body).wrap(Wrap { trim: false }), inner);
        }
        Err(error) => frame.render_widget(Paragraph::new(error.clone()), inner),
    }
}

fn ui(frame: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(2),
        ])
        .split(frame.area());

    let tabs = Tabs::new(TABS.to_vec())
        .select(app.tab)
        .block(section("Rextto"))
        .highlight_style(
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        );
    frame.render_widget(tabs, chunks[0]);

    match app.tab {
        0 => draw_status(frame, chunks[1], app),
        1 => draw_torrents(frame, chunks[1], app),
        2 => draw_events(frame, chunks[1], app),
        3 => draw_logs(frame, chunks[1], app),
        _ => draw_health(frame, chunks[1], app),
    }

    let hint = format!(
        " 1-5/tab: switch · r: refresh · c: run cycle · p: pause/resume · ↑↓: select · q: quit    {}",
        app.message
    );
    frame.render_widget(
        Paragraph::new(hint).style(Style::default().fg(Color::Gray)),
        chunks[2],
    );
}

fn setup_terminal() -> io::Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    Terminal::new(CrosstermBackend::new(stdout))
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> io::Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()
}

fn main() -> io::Result<()> {
    let base = std::env::var("REXTTO_URL").unwrap_or_else(|_| "http://127.0.0.1:5000".to_string());
    let token = std::env::var("REXTTO_API_TOKEN")
        .ok()
        .filter(|value| !value.trim().is_empty());
    let mut app = App::new(Client::new(base, token));

    let mut terminal = setup_terminal()?;
    let result = run(&mut terminal, &mut app);
    restore_terminal(&mut terminal)?;
    result
}

fn run(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    app: &mut App,
) -> io::Result<()> {
    loop {
        terminal.draw(|frame| ui(frame, app))?;

        if event::poll(Duration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        return Ok(())
                    }
                    KeyCode::Char('c') => app.run_cycle(),
                    KeyCode::Char('r') => {
                        app.refresh();
                        app.message = "refreshed".into();
                    }
                    KeyCode::Char('p') => app.toggle_selected(),
                    KeyCode::Tab | KeyCode::Right => {
                        app.tab = (app.tab + 1) % TABS.len();
                        if app.tab == 3 {
                            app.refresh_logs();
                        }
                    }
                    KeyCode::BackTab | KeyCode::Left => {
                        app.tab = (app.tab + TABS.len() - 1) % TABS.len();
                        if app.tab == 3 {
                            app.refresh_logs();
                        }
                    }
                    KeyCode::Char('1') => app.tab = 0,
                    KeyCode::Char('2') => app.tab = 1,
                    KeyCode::Char('3') => app.tab = 2,
                    KeyCode::Char('4') => app.tab = 3,
                    KeyCode::Char('5') => app.tab = 4,
                    KeyCode::Down => {
                        let count = app.torrent_list().len();
                        if count > 0 {
                            app.selected = (app.selected + 1).min(count - 1);
                        }
                    }
                    KeyCode::Up => {
                        app.selected = app.selected.saturating_sub(1);
                    }
                    _ => {}
                }
            }
        }

        if app.last_refresh.elapsed() >= REFRESH {
            app.refresh();
            app.last_refresh = Instant::now();
        }
    }
}
