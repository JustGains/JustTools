use std::collections::HashSet;
use std::io;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, TableState, Wrap};
use ratatui::{DefaultTerminal, Frame};

use super::catalog::{App, Platform};
use super::detect::{self, Detection};
use super::plan::{self, InstallPlan};

const ACCENT: Color = Color::Rgb(104, 182, 255);
const READY: Color = Color::Rgb(111, 214, 151);
const RECOMMENDED: Color = Color::Rgb(245, 184, 90);
const MUTED: Color = Color::Rgb(118, 128, 145);

#[derive(Debug)]
pub struct Selection {
    pub ids: Vec<String>,
    pub detection: Detection,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Mode {
    Normal,
    Search,
    Confirm,
    Help,
}

struct AppState {
    platform: Platform,
    apps: Vec<App>,
    detection_rx: Receiver<Detection>,
    detection: Option<Detection>,
    selected_ids: HashSet<String>,
    selected_row: usize,
    table_state: TableState,
    query: String,
    mode: Mode,
    preview: Option<InstallPlan>,
    status: String,
    started: Instant,
    should_quit: bool,
    confirmed: bool,
}

impl AppState {
    fn new(platform: Platform, apps: Vec<App>) -> Self {
        let (sender, receiver) = mpsc::channel();
        let scan_apps = apps.clone();
        thread::spawn(move || {
            let _ = sender.send(detect::scan(platform, &scan_apps));
        });
        Self::with_receiver(platform, apps, receiver)
    }

    fn with_receiver(
        platform: Platform,
        apps: Vec<App>,
        detection_rx: Receiver<Detection>,
    ) -> Self {
        Self {
            platform,
            apps,
            detection_rx,
            detection: None,
            selected_ids: HashSet::new(),
            selected_row: 0,
            table_state: TableState::default(),
            query: String::new(),
            mode: Mode::Normal,
            preview: None,
            status: "Scanning installed software in the background…".into(),
            started: Instant::now(),
            should_quit: false,
            confirmed: false,
        }
    }

    fn run(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        while !self.should_quit {
            self.receive_detection();
            terminal.draw(|frame| draw(frame, self))?;
            if event::poll(Duration::from_millis(100))?
                && let Event::Key(key) = event::read()?
                && key.kind == KeyEventKind::Press
            {
                self.handle_key(key);
            }
        }
        Ok(())
    }

    fn receive_detection(&mut self) {
        let Ok(detection) = self.detection_rx.try_recv() else {
            return;
        };
        self.selected_ids
            .retain(|id| !detection.installed(id.as_str()));
        let installed = detection.installed_count();
        self.status = if detection.warnings.is_empty() {
            format!("Scan complete — {installed} catalog app(s) already installed")
        } else {
            format!(
                "Scan complete with {} warning(s) — press ? for details",
                detection.warnings.len()
            )
        };
        self.detection = Some(detection);
    }

    fn handle_key(&mut self, key: KeyEvent) {
        match self.mode {
            Mode::Search => self.handle_search(key),
            Mode::Confirm => self.handle_confirm(key),
            Mode::Help => {
                if matches!(key.code, KeyCode::Esc | KeyCode::Char('?') | KeyCode::Enter) {
                    self.mode = Mode::Normal;
                }
            }
            Mode::Normal => self.handle_normal(key),
        }
    }

    fn handle_search(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::Enter => self.mode = Mode::Normal,
            KeyCode::Backspace => {
                self.query.pop();
                self.selected_row = 0;
            }
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.query.push(character);
                self.selected_row = 0;
            }
            _ => {}
        }
    }

    fn handle_confirm(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => {
                self.confirmed = true;
                self.should_quit = true;
            }
            KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
                self.mode = Mode::Normal;
                self.preview = None;
                self.status = "Selection kept — review or change it".into();
            }
            _ => {}
        }
    }

    fn handle_normal(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,
            KeyCode::Down | KeyCode::Char('j') => self.move_row(1),
            KeyCode::Up | KeyCode::Char('k') => self.move_row(-1),
            KeyCode::PageDown => self.move_row(8),
            KeyCode::PageUp => self.move_row(-8),
            KeyCode::Home => self.selected_row = 0,
            KeyCode::End => {
                self.selected_row = self.visible_indices().len().saturating_sub(1);
            }
            KeyCode::Tab => self.jump_section(1),
            KeyCode::BackTab => self.jump_section(-1),
            KeyCode::Char('/') => self.mode = Mode::Search,
            KeyCode::Char('c') if !self.query.is_empty() => {
                self.query.clear();
                self.selected_row = 0;
                self.status = "Search cleared".into();
            }
            KeyCode::Char(' ') => self.toggle_current(),
            KeyCode::Char('r') => self.select_recommended(),
            KeyCode::Char('a') => self.toggle_all(),
            KeyCode::Char('?') => self.mode = Mode::Help,
            KeyCode::Enter => self.prepare_confirmation(),
            _ => {}
        }
    }

    fn visible_indices(&self) -> Vec<usize> {
        let query = self.query.trim().to_ascii_lowercase();
        self.apps
            .iter()
            .enumerate()
            .filter_map(|(index, app)| {
                (query.is_empty()
                    || app.name.to_ascii_lowercase().contains(&query)
                    || app.id.contains(&query)
                    || app.description.to_ascii_lowercase().contains(&query)
                    || app.section.label().to_ascii_lowercase().contains(&query))
                .then_some(index)
            })
            .collect()
    }

    fn selected_app(&self) -> Option<&App> {
        self.visible_indices()
            .get(self.selected_row)
            .and_then(|index| self.apps.get(*index))
    }

    fn move_row(&mut self, amount: isize) {
        let maximum = self.visible_indices().len().saturating_sub(1) as isize;
        self.selected_row = (self.selected_row as isize + amount).clamp(0, maximum) as usize;
    }

    fn jump_section(&mut self, direction: isize) {
        let visible = self.visible_indices();
        let Some(current_index) = visible.get(self.selected_row) else {
            return;
        };
        let section = self.apps[*current_index].section;
        let target = if direction > 0 {
            visible
                .iter()
                .position(|index| self.apps[*index].section > section)
        } else {
            visible
                .iter()
                .enumerate()
                .rev()
                .find_map(|(row, index)| (self.apps[*index].section < section).then_some(row))
        };
        if let Some(row) = target {
            self.selected_row = row;
        }
    }

    fn toggle_current(&mut self) {
        let Some(app) = self.selected_app() else {
            return;
        };
        let id = app.id.to_owned();
        let name = app.name;
        if self
            .detection
            .as_ref()
            .is_some_and(|detection| detection.installed(&id))
        {
            self.status = format!("{name} is already installed");
            return;
        }
        if !self.selected_ids.remove(&id) {
            self.selected_ids.insert(id);
            self.status = format!("Selected {name}");
        } else {
            self.status = format!("Removed {name}");
        }
    }

    fn select_recommended(&mut self) {
        let Some(detection) = &self.detection else {
            self.status = "Let the installed-software scan finish first".into();
            return;
        };
        for app in &self.apps {
            if app.recommended && !detection.installed(app.id) {
                self.selected_ids.insert(app.id.to_owned());
            }
        }
        self.status = format!(
            "Selected {} missing recommendation(s)",
            self.selected_ids.len()
        );
    }

    fn toggle_all(&mut self) {
        let Some(detection) = &self.detection else {
            self.status = "Let the installed-software scan finish first".into();
            return;
        };
        let missing = self
            .apps
            .iter()
            .filter(|app| !detection.installed(app.id))
            .map(|app| app.id.to_owned())
            .collect::<HashSet<_>>();
        if missing.iter().all(|id| self.selected_ids.contains(id)) {
            self.selected_ids.clear();
            self.status = "Selection cleared".into();
        } else {
            self.selected_ids = missing;
            self.status = format!("Selected all {} missing app(s)", self.selected_ids.len());
        }
    }

    fn prepare_confirmation(&mut self) {
        let Some(detection) = &self.detection else {
            self.status =
                "Still scanning — installation review will unlock when it finishes".into();
            return;
        };
        if self.selected_ids.is_empty() {
            self.status = "Select at least one missing app first".into();
            return;
        }
        let ids = self.selected_ids.iter().cloned().collect::<Vec<_>>();
        match plan::build(self.platform, &self.apps, &ids, detection) {
            Ok(preview) if preview.actions.is_empty() => {
                self.status = "Everything selected is already installed".into();
            }
            Ok(preview) => {
                self.preview = Some(preview);
                self.mode = Mode::Confirm;
            }
            Err(error) => self.status = error,
        }
    }
}

pub fn choose(platform: Platform, apps: Vec<App>) -> io::Result<Option<Selection>> {
    let mut app = AppState::new(platform, apps);
    ratatui::run(|terminal| app.run(terminal))?;
    if !app.confirmed {
        return Ok(None);
    }
    let detection = app
        .detection
        .expect("confirmation is unavailable until detection completes");
    let mut ids = app.selected_ids.into_iter().collect::<Vec<_>>();
    ids.sort();
    Ok(Some(Selection { ids, detection }))
}

fn draw(frame: &mut Frame<'_>, app: &mut AppState) {
    let detail_height = if frame.area().height >= 25 { 6 } else { 4 };
    let [header, table, details, footer] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(7),
        Constraint::Length(detail_height),
        Constraint::Length(2),
    ])
    .areas(frame.area());
    draw_header(frame, app, header);
    draw_table(frame, app, table);
    draw_details(frame, app, details);
    draw_footer(frame, app, footer);
    match app.mode {
        Mode::Confirm => draw_confirmation(frame, app),
        Mode::Help => draw_help(frame, app),
        Mode::Normal | Mode::Search => {}
    }
}

fn draw_header(frame: &mut Frame<'_>, app: &AppState, area: Rect) {
    let installed = app.detection.as_ref().map_or(0, Detection::installed_count);
    let missing = app.apps.len().saturating_sub(installed);
    let scan = if app.detection.is_some() {
        Span::styled(format!(" {installed} installed  {missing} missing "), READY)
    } else {
        let frames = ["◐", "◓", "◑", "◒"];
        let frame = frames[(app.started.elapsed().as_millis() / 180) as usize % frames.len()];
        Span::styled(format!(" {frame} scanning installed apps "), ACCENT)
    };
    let mut line = vec![
        scan,
        Span::raw("  selected:"),
        Span::styled(
            app.selected_ids.len().to_string(),
            Style::default().fg(RECOMMENDED).bold(),
        ),
        Span::raw("  OS:"),
        Span::styled(app.platform.label(), Style::default().fg(ACCENT).bold()),
    ];
    if app.mode == Mode::Search || !app.query.is_empty() {
        line.extend([
            Span::raw("  /"),
            Span::styled(app.query.as_str(), Style::default().fg(Color::White).bold()),
            Span::styled(
                if app.mode == Mode::Search { "▌" } else { "" },
                Style::default().fg(ACCENT),
            ),
        ]);
    }
    frame.render_widget(
        Paragraph::new(Line::from(line)).block(
            Block::default()
                .borders(Borders::ALL)
                .title(Span::styled(" JustReady ", Style::default().bold())),
        ),
        area,
    );
}

fn draw_table(frame: &mut Frame<'_>, app: &mut AppState, area: Rect) {
    let visible = app.visible_indices();
    let rows = visible.iter().map(|index| {
        let entry = &app.apps[*index];
        let installed = app
            .detection
            .as_ref()
            .is_some_and(|detection| detection.installed(entry.id));
        let selected = app.selected_ids.contains(entry.id);
        let (marker, style, state) = if installed {
            ("✓", Style::default().fg(READY), "installed")
        } else if selected {
            ("■", Style::default().fg(RECOMMENDED).bold(), "selected")
        } else if app.detection.is_none() {
            ("…", Style::default().fg(MUTED), "checking")
        } else {
            ("□", Style::default().fg(MUTED), "available")
        };
        let name = if entry.recommended {
            format!("{}  ★", entry.name)
        } else {
            entry.name.to_owned()
        };
        let row_style = if installed {
            Style::default().fg(MUTED)
        } else {
            Style::default()
        };
        Row::new(vec![
            Cell::from(marker).style(style),
            Cell::from(entry.section.label()),
            Cell::from(name),
            Cell::from(entry.source.label()),
            Cell::from(state).style(style),
        ])
        .style(row_style)
    });
    let title = format!(
        " Apps  {} visible / {} for {} ",
        visible.len(),
        app.apps.len(),
        app.platform.label()
    );
    let table = Table::new(
        rows,
        [
            Constraint::Length(2),
            Constraint::Length(23),
            Constraint::Min(20),
            Constraint::Length(23),
            Constraint::Length(11),
        ],
    )
    .header(
        Row::new(["", "Section", "App", "Installer", "State"])
            .style(Style::default().fg(Color::White).bold())
            .bottom_margin(1),
    )
    .block(Block::default().borders(Borders::ALL).title(title))
    .row_highlight_style(
        Style::default()
            .bg(Color::Rgb(36, 48, 64))
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    )
    .highlight_symbol("› ");
    if !visible.is_empty() {
        app.selected_row = app.selected_row.min(visible.len() - 1);
    }
    app.table_state
        .select((!visible.is_empty()).then_some(app.selected_row));
    frame.render_stateful_widget(table, area, &mut app.table_state);
}

fn draw_details(frame: &mut Frame<'_>, app: &AppState, area: Rect) {
    let block = Block::default().borders(Borders::ALL).title(" Details ");
    let Some(entry) = app.selected_app() else {
        frame.render_widget(
            Paragraph::new("No apps match this search")
                .alignment(Alignment::Center)
                .block(block),
            area,
        );
        return;
    };
    let recommendation = if entry.recommended {
        Span::styled("RECOMMENDED", Style::default().fg(RECOMMENDED).bold())
    } else {
        Span::styled("OPTIONAL", Style::default().fg(MUTED).bold())
    };
    let lines = vec![
        Line::from(vec![
            Span::styled(entry.name, Style::default().bold()),
            Span::raw("  "),
            recommendation,
            Span::styled(format!("  id:{}", entry.id), Style::default().fg(MUTED)),
        ]),
        Line::from(entry.description),
        Line::from(vec![
            Span::styled("Source: ", Style::default().fg(MUTED)),
            Span::raw(entry.source.label()),
        ]),
    ];
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: true }).block(block),
        area,
    );
}

fn draw_footer(frame: &mut Frame<'_>, app: &AppState, area: Rect) {
    let keys = match app.mode {
        Mode::Search => "type to filter  Enter apply  Esc close",
        _ => {
            "↑↓ move  Space select  r recommended  a all  / search  Tab section  Enter install  ? help  q quit"
        }
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(format!(" {} ", app.status), Style::default().fg(ACCENT)),
            Span::styled(format!("  {keys}"), Style::default().fg(MUTED)),
        ])),
        area,
    );
}

fn draw_confirmation(frame: &mut Frame<'_>, app: &AppState) {
    let Some(plan) = &app.preview else {
        return;
    };
    let area = centered(frame.area(), 74, 68);
    frame.render_widget(Clear, area);
    let app_names = plan::names_for_ids(&app.apps, &plan.app_ids);
    let mut lines = vec![
        Line::styled(
            format!(
                "Install {} app(s) in {} step(s)?",
                app_names.len(),
                plan.actions.len()
            ),
            Style::default().fg(Color::White).bold(),
        ),
        Line::raw(""),
    ];
    for name in app_names.iter().take(10) {
        lines.push(Line::from(vec![
            Span::styled("  • ", Style::default().fg(RECOMMENDED)),
            Span::raw(*name),
        ]));
    }
    if app_names.len() > 10 {
        lines.push(Line::styled(
            format!("  … and {} more", app_names.len() - 10),
            Style::default().fg(MUTED),
        ));
    }
    if !plan.dependency_ids.is_empty() {
        lines.push(Line::raw(""));
        lines.push(Line::styled(
            format!(
                "Dependencies added automatically: {}",
                plan::names_for_ids(&app.apps, &plan.dependency_ids).join(", ")
            ),
            Style::default().fg(ACCENT),
        ));
    }
    lines.extend([
        Line::raw(""),
        Line::styled(
            "The TUI closes before installers run, so prompts and progress remain visible.",
            Style::default().fg(MUTED),
        ),
        Line::raw(""),
        Line::styled(
            "Enter / y install    Esc / n go back",
            Style::default().fg(READY).bold(),
        ),
    ]);
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: true }).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Review installation "),
        ),
        area,
    );
}

fn draw_help(frame: &mut Frame<'_>, app: &AppState) {
    let area = centered(frame.area(), 76, 76);
    frame.render_widget(Clear, area);
    let mut lines = vec![
        Line::styled(
            "JustReady chooses the native installer for this OS.",
            Style::default().bold(),
        ),
        Line::raw(""),
        Line::raw("↑/↓ or j/k       move without list jumping during scans"),
        Line::raw("Space            select or clear one missing app"),
        Line::raw("r                select missing recommended apps"),
        Line::raw("a                select/clear every missing app"),
        Line::raw("/                search names, ids, descriptions, and sections"),
        Line::raw("Tab / Shift-Tab  jump between sections"),
        Line::raw("Enter            review the complete dependency-aware plan"),
        Line::raw("q                quit without changing the system"),
        Line::raw(""),
        Line::styled(
            "Installed apps are detected once in the background and cannot be selected.",
            Style::default().fg(MUTED),
        ),
    ];
    if let Some(detection) = &app.detection
        && !detection.warnings.is_empty()
    {
        lines.push(Line::raw(""));
        lines.push(Line::styled(
            "Scan notes:",
            Style::default().fg(RECOMMENDED).bold(),
        ));
        for warning in &detection.warnings {
            lines.push(Line::raw(format!("• {warning}")));
        }
    }
    lines.extend([
        Line::raw(""),
        Line::styled(
            "Esc / ? / Enter closes help",
            Style::default().fg(READY).bold(),
        ),
    ]);
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: true })
            .block(Block::default().borders(Borders::ALL).title(" Help ")),
        area,
    );
}

fn centered(area: Rect, width_percent: u16, height_percent: u16) -> Rect {
    let [vertical] = Layout::vertical([Constraint::Percentage(height_percent)])
        .flex(ratatui::layout::Flex::Center)
        .areas(area);
    let [horizontal] = Layout::horizontal([Constraint::Percentage(width_percent)])
        .flex(ratatui::layout::Flex::Center)
        .areas(vertical);
    horizontal
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;
    use crate::commands::ready::detect::SystemState;

    #[test]
    fn initial_frame_renders_before_detection_finishes() {
        let apps = super::super::catalog::for_platform(Platform::Windows);
        let (_sender, receiver) = mpsc::channel();
        let mut state = AppState::with_receiver(Platform::Windows, apps, receiver);
        let backend = TestBackend::new(100, 28);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let rendered = terminal.backend().to_string();
        assert!(rendered.contains("JustReady"));
        assert!(rendered.contains("scanning installed apps"));
    }

    #[test]
    fn completed_scan_and_confirmation_render_without_selecting_installed_apps() {
        let apps = super::super::catalog::for_platform(Platform::Windows);
        let (sender, receiver) = mpsc::channel();
        let mut state = AppState::with_receiver(Platform::Windows, apps, receiver);
        sender
            .send(Detection::test_with(
                SystemState {
                    winget: true,
                    ..SystemState::default()
                },
                &["git"],
            ))
            .unwrap();
        state.receive_detection();
        state.select_recommended();
        assert!(!state.selected_ids.contains("git"));
        state.prepare_confirmation();
        assert_eq!(state.mode, Mode::Confirm);

        let backend = TestBackend::new(110, 34);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        assert!(
            terminal
                .backend()
                .to_string()
                .contains("Review installation")
        );
    }
}
