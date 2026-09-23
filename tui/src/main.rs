//! Rextto TUI/CLI — terminal client for a running Rextto daemon.
//!
//! Two modes:
//! * **text commands** (works in any shell, scriptable, no full-screen needed):
//!   `rextto-tui status|torrents|events|logs [n]|health|cycle [domain]|pause <hash>|resume <hash>|rename-all|help`
//! * **interactive** full-screen view, started with no arguments on a real
//!   terminal (or with `-i`): Status, Torrents, Activity, Logs, Health.
//!
//! It only talks to the daemon's HTTP API and never touches the databases.

use std::{
    io::{self, IsTerminal},
    process::ExitCode,
    time::{Duration, Instant},
};

use serde_json::Value;

const DEFAULT_URL: &str = "http://127.0.0.1:5000";

struct Client {
    base: String,
    token: Option<String>,
}

impl Client {
    fn from_env() -> Self {
        let base =
            std::env::var("REXTTO_URL").unwrap_or_else(|_| DEFAULT_URL.to_string());
        let token = std::env::var("REXTTO_API_TOKEN")
            .ok()
            .filter(|value| !value.trim().is_empty());
        Self {
            base: base.trim_end_matches('/').to_string(),
            token,
        }
    }

    fn request(&self, method: &str, path: &str) -> Result<Value, String> {
        let url = format!("{}{}", self.base, path);
        let mut request = match method {
            "POST" => ureq::post(&url).set("Content-Type", "application/json"),
            _ => ureq::get(&url),
        }
        .timeout(Duration::from_secs(20));
        if let Some(token) = &self.token {
            request = request.set("x-rextto-token", token);
        }
        let response = if method == "POST" {
            request.send_json(serde_json::json!({}))
        } else {
            request.call()
        };
        response
            .map_err(|error| error.to_string())?
            .into_json::<Value>()
            .map_err(|error| error.to_string())
    }

    fn get(&self, path: &str) -> Result<Value, String> {
        self.request("GET", path)
    }

    fn post(&self, path: &str) -> Result<Value, String> {
        self.request("POST", path)
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

fn shorten(name: &str, width: usize) -> String {
    if name.chars().count() <= width {
        name.to_string()
    } else {
        let mut out = name.chars().take(width.saturating_sub(1)).collect::<String>();
        out.push('…');
        out
    }
}

// ---------------------------------------------------------------------------
// Text commands (shell / scripting mode)
// ---------------------------------------------------------------------------

fn print_status(client: &Client) -> Result<(), String> {
    let status = client.get("/api/status")?;
    let torrents = status.get("torrent_stats").cloned().unwrap_or(Value::Null);
    let cycle = status.get("last_cycle").cloned().unwrap_or(Value::Null);
    let seen = status.get("seen").cloned().unwrap_or(Value::Null);
    let active = status.get("active").and_then(Value::as_bool).unwrap_or(false);
    println!(
        "{} v{} — {}",
        if text(&status, "name").is_empty() {
            "rextto".to_string()
        } else {
            text(&status, "name")
        },
        text(&status, "version"),
        if active { "ACTIVE" } else { "PAUSED" },
    );
    if status.get("dry_run").and_then(Value::as_bool).unwrap_or(false) {
        println!("dry-run: yes");
    }
    println!(
        "torrents: {} ({} downloading, {} queued, {} seeding)",
        text(&torrents, "count"),
        text(&torrents, "downloading"),
        text(&torrents, "queued"),
        text(&torrents, "seeding"),
    );
    println!(
        "last cycle: scraped {} | candidates {} | downloads {} | errors {}",
        text(&cycle, "scraped"),
        text(&cycle, "candidates"),
        text(&cycle, "downloads_started"),
        text(&cycle, "errors"),
    );
    println!(
        "seen in feeds: groups {} · movies {} · series {}",
        text(&seen, "groups"),
        text(&seen, "movies"),
        text(&seen, "series"),
    );
    Ok(())
}

fn print_torrents(client: &Client) -> Result<(), String> {
    let torrents = client.get("/api/torrents")?;
    let items = torrents.as_array().cloned().unwrap_or_default();
    println!(
        "{:<10} {:<12} {:>6} {:>10} {:>12} {:>10}  {}",
        "HASH", "STATE", "PROG", "DONE", "DOWN", "UP", "NAME"
    );
    for torrent in items {
        let progress = torrent.get("progress").and_then(Value::as_f64).unwrap_or(0.0);
        let rate = |key: &str| {
            human_bytes(torrent.get(key).and_then(Value::as_u64).unwrap_or(0) as f64)
        };
        println!(
            "{:<10} {:<12} {:>5.1}% {:>10} {:>10}/s {:>8}/s  {}",
            shorten(
                torrent.get("hash").and_then(Value::as_str).unwrap_or_default(),
                9
            ),
            shorten(&text(&torrent, "state"), 12),
            progress,
            human_bytes(torrent.get("total_done").and_then(Value::as_i64).unwrap_or(0) as f64),
            rate("download_rate"),
            rate("upload_rate"),
            shorten(&text(&torrent, "name"), 60),
        );
    }
    Ok(())
}

fn print_events(client: &Client) -> Result<(), String> {
    let events = client.get("/api/torrent-events")?;
    for event in events.as_array().cloned().unwrap_or_default() {
        println!(
            "{:<18} {:<10} {}",
            shorten(&text(&event, "kind"), 18),
            shorten(
                event.get("hash").and_then(Value::as_str).unwrap_or_default(),
                9
            ),
            text(&event, "name"),
        );
    }
    Ok(())
}

fn print_logs(client: &Client, limit: usize) -> Result<(), String> {
    let logs = client.get(&format!("/api/logs?limit={limit}"))?;
    for line in logs
        .get("items")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
    {
        if let Some(line) = line.as_str() {
            println!("{line}");
        }
    }
    Ok(())
}

fn print_health(client: &Client) -> Result<(), String> {
    let health = client.get("/api/health")?;
    println!(
        "{}",
        serde_json::to_string_pretty(&health).unwrap_or_default()
    );
    Ok(())
}

fn run_cycle(client: &Client, domain: Option<&str>) -> Result<(), String> {
    let path = match domain {
        Some(domain) => format!("/api/run-now?domain={domain}"),
        None => "/api/run-now".to_string(),
    };
    let result = client.post(&path)?;
    println!(
        "cycle requested: {}",
        serde_json::to_string(&result).unwrap_or_default()
    );
    Ok(())
}

fn torrent_action(client: &Client, action: &str, hash: &str) -> Result<(), String> {
    let result = client.post(&format!("/api/torrents/{hash}/{action}"))?;
    println!(
        "{action} {hash}: {}",
        serde_json::to_string(&result).unwrap_or_default()
    );
    Ok(())
}

fn rename_all(client: &Client) -> Result<(), String> {
    let result = client.post("/api/rename-all")?;
    println!(
        "rename requested: {}",
        serde_json::to_string(&result).unwrap_or_default()
    );
    // Poll until finished so the command is useful from a script.
    let mut last = String::new();
    loop {
        std::thread::sleep(Duration::from_secs(2));
        let progress = match client.get("/api/rename-progress") {
            Ok(value) => value.get("progress").cloned().unwrap_or(Value::Null),
            Err(_) => break,
        };
        let line = format!(
            "{}/{} {} {}",
            text(&progress, "current"),
            text(&progress, "total"),
            text(&progress, "series"),
            text(&progress, "message"),
        );
        if line != last {
            println!("{line}");
            last = line;
        }
        if !progress
            .get("running")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            break;
        }
    }
    Ok(())
}

fn usage() {
    println!(
        "rextto-tui — Rextto terminal client\n\n\
         Usage:\n  \
         rextto-tui                      interactive full-screen view (on a terminal)\n  \
         rextto-tui status               daemon summary\n  \
         rextto-tui torrents             torrent session table\n  \
         rextto-tui events               recent torrent events\n  \
         rextto-tui logs [n]             last log lines (default 80)\n  \
         rextto-tui health               health report (JSON)\n  \
         rextto-tui cycle [domain]       run a cycle (domain: series|movies|comics)\n  \
         rextto-tui pause <hash>         pause a torrent\n  \
         rextto-tui resume <hash>        resume a torrent\n  \
         rextto-tui rename-all           rename the library in the background\n  \
         rextto-tui -i | --interactive   force the full-screen view\n  \
         rextto-tui help                 this help\n\n\
         Environment:\n  \
         REXTTO_URL         daemon base URL (default {DEFAULT_URL})\n  \
         REXTTO_API_TOKEN   API token, if configured"
    );
}

fn run_command(client: &Client, command: &str, args: &[String]) -> Result<(), String> {
    match command {
        "status" => print_status(client),
        "torrents" => print_torrents(client),
        "events" => print_events(client),
        "logs" => {
            let limit = args
                .first()
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(80)
                .clamp(1, 2000);
            print_logs(client, limit)
        }
        "health" => print_health(client),
        "cycle" => run_cycle(client, args.first().map(String::as_str)),
        "pause" => match args.first() {
            Some(hash) => torrent_action(client, "pause", hash),
            None => Err("usage: rextto-tui pause <hash>".into()),
        },
        "resume" => match args.first() {
            Some(hash) => torrent_action(client, "resume", hash),
            None => Err("usage: rextto-tui resume <hash>".into()),
        },
        "rename-all" => rename_all(client),
        "help" | "-h" | "--help" => {
            usage();
            Ok(())
        }
        other => Err(format!("unknown command '{other}' (try: rextto-tui help)")),
    }
}

// ---------------------------------------------------------------------------
// Interactive full-screen view
// ---------------------------------------------------------------------------

mod interactive {
    use super::*;
    use crossterm::{
        event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
        execute,
        terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    };
    use ratatui::{
        backend::CrosstermBackend,
        layout::{Constraint, Direction, Layout, Rect},
        style::{Color, Modifier, Style},
        text::{Line, Span},
        widgets::{Block, Borders, Cell, Paragraph, Row, Table, Tabs, Wrap},
        Frame, Terminal,
    };

    const TABS: [&str; 5] = ["Status", "Torrents", "Activity", "Logs", "Health"];

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
        message: String,
    }

    impl App {
        fn new(client: Client) -> Self {
            Self {
                client,
                tab: 0,
                status: Err("loading…".into()),
                torrents: Err("loading…".into()),
                events: Err("loading…".into()),
                logs: Err("loading…".into()),
                health: Err("loading…".into()),
                selected: 0,
                last_refresh: Instant::now() - Duration::from_secs(10),
                message: String::new(),
            }
        }

        fn refresh(&mut self) {
            self.status = self.client.get("/api/status");
            self.torrents = self.client.get("/api/torrents");
            self.events = self.client.get("/api/torrent-events");
            self.health = self.client.get("/api/health");
            self.logs = self.client.get("/api/logs?limit=200");
            let count = self.torrent_list().len();
            if count == 0 {
                self.selected = 0;
            } else if self.selected >= count {
                self.selected = count - 1;
            }
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

    fn section(title: &str) -> Block<'static> {
        Block::default().borders(Borders::ALL).title(title.to_string())
    }

    fn draw_status(frame: &mut Frame, area: Rect, app: &App) {
        let block = section("Overview");
        let inner = block.inner(area);
        frame.render_widget(block, area);
        match &app.status {
            Ok(status) => {
                let torrents = status.get("torrent_stats").cloned().unwrap_or(Value::Null);
                let cycle = status.get("last_cycle").cloned().unwrap_or(Value::Null);
                let seen = status.get("seen").cloned().unwrap_or(Value::Null);
                let active = status.get("active").and_then(Value::as_bool).unwrap_or(false);
                let lines = vec![
                    Line::from(vec![
                        Span::styled(
                            format!(" {} ", text(status, "name")),
                            Style::default().add_modifier(Modifier::BOLD),
                        ),
                        Span::raw(format!("v{}  ", text(status, "version"))),
                        Span::styled(
                            if active { "ACTIVE" } else { "PAUSED" },
                            Style::default().fg(if active { Color::Green } else { Color::Yellow }),
                        ),
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
                    Line::from(format!(
                        "Seen in feeds: groups {} · movies {} · series {}",
                        text(&seen, "groups"),
                        text(&seen, "movies"),
                        text(&seen, "series"),
                    )),
                ];
                frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
            }
            Err(error) => frame.render_widget(
                Paragraph::new(format!(
                    "cannot reach daemon: {error}\n\nset REXTTO_URL / REXTTO_API_TOKEN"
                ))
                .style(Style::default().fg(Color::Red))
                .wrap(Wrap { trim: false }),
                inner,
            ),
        }
    }

    fn draw_torrents(frame: &mut Frame, area: Rect, app: &App) {
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
            let row = Row::new(vec![
                Cell::from(shorten(&text(torrent, "name"), 60)),
                Cell::from(text(torrent, "state")),
                Cell::from(format!("{progress:.1}%")),
                Cell::from(format!("{} / {}", human_bytes(done), human_bytes(total))),
                Cell::from(format!(
                    "↓{}/s ↑{}/s",
                    human_bytes(down as f64),
                    human_bytes(up as f64)
                )),
            ]);
            if index == app.selected {
                row.style(
                    Style::default()
                        .bg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD),
                )
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

    fn draw_events(frame: &mut Frame, area: Rect, app: &App) {
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
                                    shorten(
                                        item.get("hash").and_then(Value::as_str).unwrap_or_default(),
                                        9
                                    ),
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

    fn draw_logs(frame: &mut Frame, area: Rect, app: &App) {
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

    fn draw_health(frame: &mut Frame, area: Rect, app: &App) {
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
            .highlight_style(Style::default().fg(Color::Green).add_modifier(Modifier::BOLD));
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

    pub fn run(client: Client) -> io::Result<()> {
        let mut app = App::new(client);
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let mut terminal = Terminal::new(CrosstermBackend::new(stdout))?;

        let result = (|| -> io::Result<()> {
            loop {
                terminal.draw(|frame| ui(frame, &app))?;
                if event::poll(Duration::from_millis(200))? {
                    if let Event::Key(key) = event::read()? {
                        if key.kind != KeyEventKind::Press {
                            continue;
                        }
                        match key.code {
                            KeyCode::Char('q') | KeyCode::Esc => break,
                            KeyCode::Char('c')
                                if key.modifiers.contains(KeyModifiers::CONTROL) =>
                            {
                                break
                            }
                            KeyCode::Char('c') => app.run_cycle(),
                            KeyCode::Char('r') => {
                                app.refresh();
                                app.message = "refreshed".into();
                            }
                            KeyCode::Char('p') => app.toggle_selected(),
                            KeyCode::Tab | KeyCode::Right => {
                                app.tab = (app.tab + 1) % TABS.len();
                            }
                            KeyCode::BackTab | KeyCode::Left => {
                                app.tab = (app.tab + TABS.len() - 1) % TABS.len();
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
                            KeyCode::Up => app.selected = app.selected.saturating_sub(1),
                            _ => {}
                        }
                    }
                }
                if app.last_refresh.elapsed() >= Duration::from_secs(2) {
                    app.refresh();
                    app.last_refresh = Instant::now();
                }
            }
            Ok(())
        })();

        disable_raw_mode()?;
        execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
        terminal.show_cursor()?;
        result
    }
}

fn main() -> ExitCode {
    // Behave like a normal shell tool when piped into `head`: die on SIGPIPE
    // instead of panicking with a "Broken pipe" message.
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }

    let client = Client::from_env();
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let first = args.first().map(String::as_str);
    let force_interactive = matches!(first, Some("-i" | "--interactive"));
    let interactive = force_interactive || (args.is_empty() && io::stdout().is_terminal());

    if interactive {
        if !io::stdout().is_terminal() {
            eprintln!("not a terminal: use a text command, e.g. `rextto-tui status`");
            return ExitCode::from(2);
        }
        return match interactive::run(client) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("tui error: {error}");
                ExitCode::from(1)
            }
        };
    }

    let command = first.unwrap_or("status");
    let rest: &[String] = if args.len() > 1 { &args[1..] } else { &[] };
    match run_command(&client, command, rest) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::from(1)
        }
    }
}
