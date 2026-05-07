use crate::NotificationConfig;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState},
    Frame, Terminal,
};
use std::{io, path::PathBuf, process::Command};

pub fn run_tui(notifications_path: PathBuf) -> io::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let app = App::new(notifications_path);
    let res = run_app(&mut terminal, app);

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        eprintln!("TUI error: {err:?}");
    }
    Ok(())
}

struct App {
    notifications_path: PathBuf,
    notifications: Vec<NotificationConfig>,
    table_state: TableState,
    selected_index: usize,
    status_message: String,
}

impl App {
    fn new(notifications_path: PathBuf) -> Self {
        let notifications = load_notifications(&notifications_path);
        let len = notifications.len();
        let mut table_state = TableState::default();
        if len > 0 {
            table_state.select(Some(0));
        }
        App {
            notifications_path,
            notifications,
            table_state,
            selected_index: 0,
            status_message: String::new(),
        }
    }

    fn next(&mut self) {
        if self.notifications.is_empty() {
            return;
        }
        let i = self
            .selected_index
            .saturating_add(1)
            .min(self.notifications.len() - 1);
        self.selected_index = i;
        self.table_state.select(Some(i));
    }

    fn prev(&mut self) {
        if self.notifications.is_empty() {
            return;
        }
        let i = self.selected_index.saturating_sub(1);
        self.selected_index = i;
        self.table_state.select(Some(i));
    }

    fn reload(&mut self) {
        self.notifications = load_notifications(&self.notifications_path);
        if self.selected_index >= self.notifications.len() && !self.notifications.is_empty() {
            self.selected_index = self.notifications.len() - 1;
            self.table_state.select(Some(self.selected_index));
        } else if self.notifications.is_empty() {
            self.selected_index = 0;
            self.table_state.select(None);
        } else {
            self.table_state.select(Some(self.selected_index));
        }
    }

    fn open_editor(&mut self) {
        const SAFE_EDITORS: &[&str] = &["vi", "vim", "nvim", "nano", "emacs", "code", "hx", "helix"];

        let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vi".to_string());
        let editor_name = std::path::Path::new(&editor)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        if !SAFE_EDITORS.contains(&editor_name) {
            self.status_message = format!(
                "Editor '{editor}' is not in the safe list. Allowed: {}",
                SAFE_EDITORS.join(", ")
            );
            return;
        }
        let result = Command::new(&editor)
            .arg(self.notifications_path.to_str().unwrap_or(""))
            .status();

        match result {
            Ok(status) if status.success() => {
                self.reload();
                self.status_message = format!("Editor '{editor}' closed. Notifications reloaded.");
            }
            Ok(status) => {
                self.status_message = format!(
                    "Editor '{editor}' exited with code {}.",
                    status.code().unwrap_or(-1)
                );
            }
            Err(e) => {
                self.status_message = format!("Failed to launch editor '{editor}': {e}");
            }
        }
    }
}

fn load_notifications(path: &PathBuf) -> Vec<NotificationConfig> {
    match std::fs::read_to_string(path) {
        Ok(data) => serde_json::from_str(&data).unwrap_or_else(|e| {
            eprintln!("Warning: Failed to parse notifications file '{}': {}", path.display(), e);
            Vec::new()
        }),
        Err(e) => {
            eprintln!("Warning: Failed to read notifications file '{}': {}", path.display(), e);
            Vec::new()
        }
    }
}

fn run_app(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, mut app: App) -> io::Result<()> {
    loop {
        terminal.draw(|f| ui(f, &mut app))?;

        if let Event::Key(key) = event::read()? {
            if key.kind == KeyEventKind::Press {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                    KeyCode::Char('j') | KeyCode::Down => app.next(),
                    KeyCode::Char('k') | KeyCode::Up => app.prev(),
                    KeyCode::Char('e') => {
                        disable_raw_mode()?;
                        execute!(
                            terminal.backend_mut(),
                            LeaveAlternateScreen,
                            DisableMouseCapture
                        )?;
                        terminal.show_cursor()?;

                        app.open_editor();

                        enable_raw_mode()?;
                        execute!(
                            terminal.backend_mut(),
                            EnterAlternateScreen,
                            EnableMouseCapture
                        )?;
                        terminal.clear()?;
                    }
                    KeyCode::Char('r') => {
                        app.reload();
                        app.status_message = "Notifications reloaded.".to_string();
                    }
                    _ => {}
                }
            }
        }
    }
}

fn ui(f: &mut Frame, app: &mut App) {
    let main_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(6),
            Constraint::Length(12),
            Constraint::Length(3),
        ])
        .split(f.area());

    let header = Paragraph::new(Line::from(vec![
        Span::styled(
            "Pushel Notification Manager",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            app.notifications_path.display().to_string(),
            Style::default().fg(Color::DarkGray),
        ),
    ]))
    .block(Block::default().borders(Borders::ALL));
    f.render_widget(header, main_layout[0]);

    render_table(f, app, main_layout[1]);
    render_details(f, app, main_layout[2]);
    render_help(f, app, main_layout[3]);
}

fn render_table(f: &mut Frame, app: &mut App, area: Rect) {
    let widths = [
        Constraint::Length(4),
        Constraint::Length(18),
        Constraint::Min(20),
        Constraint::Length(10),
        Constraint::Length(10),
    ];

    let header_cells = ["#", "Title", "Message", "Interval", "Urgency"]
        .iter()
        .map(|h| Cell::from(*h).style(Style::default().add_modifier(Modifier::BOLD)));

    let header = Row::new(header_cells)
        .style(Style::default().fg(Color::White).bg(Color::DarkGray))
        .height(1);

    let rows = app.notifications.iter().enumerate().map(|(i, n)| {
        let msg = truncate_str(&n.message, 35);
        let urgency = n.urgency.as_deref().unwrap_or("-");
        let urgency_style = match urgency {
            "critical" => Style::default().fg(Color::Red),
            "normal" => Style::default().fg(Color::Yellow),
            _ => Style::default().fg(Color::Green),
        };

        Row::new(vec![
            Cell::from(format!("{}", i + 1)),
            Cell::from(n.title.as_deref().unwrap_or("Erinnerung")),
            Cell::from(msg),
            Cell::from(n.interval.as_str()),
            Cell::from(urgency).style(urgency_style),
        ])
        .height(1)
    });

    let t = Table::new(rows, widths)
        .header(header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Notifications"),
        )
        .highlight_style(
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");

    f.render_stateful_widget(t, area, &mut app.table_state);
}

fn render_details(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!("Details: #{}", app.selected_index + 1));

    if app.notifications.is_empty() || app.selected_index >= app.notifications.len() {
        let p = Paragraph::new("No notification selected")
            .block(block)
            .style(Style::default().fg(Color::DarkGray));
        f.render_widget(p, area);
        return;
    }

    let n = &app.notifications[app.selected_index];
    let lines = vec![
        Line::from(vec![
            Span::styled(
                "  Title:        ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw(n.title.as_deref().unwrap_or("-")),
        ]),
        Line::from(vec![
            Span::styled(
                "  Message:      ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw(&n.message),
        ]),
        Line::from(vec![
            Span::styled(
                "  Interval:     ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw(&n.interval),
        ]),
        Line::from(vec![
            Span::styled(
                "  Urgency:      ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw(n.urgency.as_deref().unwrap_or("-")),
        ]),
        Line::from(vec![
            Span::styled(
                "  Expire Time:  ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw(
                n.expire_time
                    .map(|t| t.to_string())
                    .unwrap_or_else(|| "-".to_string()),
            ),
        ]),
        Line::from(vec![
            Span::styled(
                "  App Name:     ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw(n.app_name.as_deref().unwrap_or("-")),
        ]),
        Line::from(vec![
            Span::styled(
                "  Icon:         ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw(n.icon.as_deref().unwrap_or("-")),
        ]),
        Line::from(vec![
            Span::styled(
                "  Category:     ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw(n.category.as_deref().unwrap_or("-")),
        ]),
        Line::from(vec![
            Span::styled(
                "  Transient:    ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw(
                n.transient
                    .map(|t| t.to_string())
                    .unwrap_or_else(|| "-".to_string()),
            ),
        ]),
    ];

    let p = Paragraph::new(lines).block(block);
    f.render_widget(p, area);
}

fn render_help(f: &mut Frame, app: &App, area: Rect) {
    let mut spans = vec![
        Span::styled("[↑↓/jk] Navigate", Style::default().fg(Color::Cyan)),
        Span::raw("  "),
        Span::styled("[e] Edit in $EDITOR", Style::default().fg(Color::Cyan)),
        Span::raw("  "),
        Span::styled("[r] Reload", Style::default().fg(Color::Cyan)),
        Span::raw("  "),
        Span::styled("[q/Esc] Quit", Style::default().fg(Color::Cyan)),
    ];

    if !app.status_message.is_empty() {
        spans.push(Span::raw("  |  "));
        spans.push(Span::styled(
            &app.status_message,
            Style::default().fg(Color::Yellow),
        ));
    }

    let p = Paragraph::new(Line::from(spans)).block(Block::default().borders(Borders::ALL));
    f.render_widget(p, area);
}

fn truncate_str(s: &str, max_len: usize) -> String {
    if s.chars().count() > max_len {
        let mut truncated: String = s.chars().take(max_len.saturating_sub(1)).collect();
        truncated.push('…');
        truncated
    } else {
        s.to_string()
    }
}
