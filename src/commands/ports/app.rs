use std::{
    io,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{DefaultTerminal, widgets::TableState};

use super::{
    cache::{HistoryStore, KnownServer},
    model::{LaunchRecipe, ServerInfo},
    scan::{ServerScanner, terminate_server},
    ui,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Focus {
    Active,
    Recent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum View {
    Development,
    All,
}

impl View {
    fn toggle(self) -> Self {
        match self {
            Self::Development => Self::All,
            Self::All => Self::Development,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Development => "dev servers",
            Self::All => "all listeners",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mode {
    Normal,
    Search,
    ConfirmKill,
    Help,
}

pub struct App {
    scanner: ServerScanner,
    history_store: HistoryStore,
    pub servers: Vec<ServerInfo>,
    pub recent: Vec<KnownServer>,
    pub selected: usize,
    pub recent_selected: usize,
    pub focus: Focus,
    pub view: View,
    pub mode: Mode,
    pub query: String,
    pub table_state: TableState,
    pub recent_table_state: TableState,
    pub pending_kill: Option<ServerInfo>,
    pub status: String,
    status_at: Instant,
    last_refresh: Instant,
    should_quit: bool,
}

impl App {
    pub fn new(mut scanner: ServerScanner, show_all: bool) -> anyhow::Result<Self> {
        let servers = scanner.scan()?;
        let mut history_store = HistoryStore::load()?;
        history_store.record(&servers)?;
        let recent = history_store.offline(&servers);
        Ok(Self {
            scanner,
            history_store,
            servers,
            recent,
            selected: 0,
            recent_selected: 0,
            focus: Focus::Active,
            view: if show_all {
                View::All
            } else {
                View::Development
            },
            mode: Mode::Normal,
            query: String::new(),
            table_state: TableState::default(),
            recent_table_state: TableState::default(),
            pending_kill: None,
            status: "Detected servers are saved automatically — press ? for shortcuts".into(),
            status_at: Instant::now(),
            last_refresh: Instant::now(),
            should_quit: false,
        })
    }

    pub fn run(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        while !self.should_quit {
            terminal.draw(|frame| ui::draw(frame, self))?;
            let refresh_interval = Duration::from_secs(2);
            let timeout = refresh_interval
                .saturating_sub(self.last_refresh.elapsed())
                .min(Duration::from_millis(200));
            if event::poll(timeout)?
                && let Event::Key(key) = event::read()?
            {
                self.handle_key(key);
            }
            if self.last_refresh.elapsed() >= refresh_interval
                && matches!(self.mode, Mode::Normal | Mode::Search)
            {
                self.refresh_now(false);
            }
        }
        Ok(())
    }

    pub fn counts(&self) -> (usize, usize) {
        (
            self.servers
                .iter()
                .filter(|server| server.is_dev_server)
                .count(),
            self.servers.len(),
        )
    }

    pub fn visible_indices(&self) -> Vec<usize> {
        let query = self.query.to_lowercase();
        self.servers
            .iter()
            .enumerate()
            .filter(|(_, server)| self.view == View::All || server.is_dev_server)
            .filter(|(_, server)| query.is_empty() || server.searchable_text().contains(&query))
            .map(|(index, _)| index)
            .collect()
    }

    pub fn selected_server(&self) -> Option<&ServerInfo> {
        let visible = self.visible_indices();
        visible
            .get(self.selected)
            .and_then(|index| self.servers.get(*index))
    }

    pub fn selected_recent(&self) -> Option<&KnownServer> {
        let visible = self.recent_visible_indices();
        visible
            .get(self.recent_selected)
            .and_then(|index| self.recent.get(*index))
    }

    pub fn recent_visible_indices(&self) -> Vec<usize> {
        let query = self.query.to_lowercase();
        self.recent
            .iter()
            .enumerate()
            .filter(|(_, server)| query.is_empty() || server.searchable_text().contains(&query))
            .map(|(index, _)| index)
            .collect()
    }

    pub fn status_text(&self) -> &str {
        if self.status_at.elapsed() > Duration::from_secs(8) {
            ""
        } else {
            &self.status
        }
    }

    fn handle_key(&mut self, key: KeyEvent) {
        if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            return;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return;
        }
        match self.mode {
            Mode::Normal => self.handle_normal_key(key),
            Mode::Search => self.handle_search_key(key),
            Mode::ConfirmKill => self.handle_confirm_kill_key(key),
            Mode::Help => {
                if matches!(
                    key.code,
                    KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q')
                ) {
                    self.mode = Mode::Normal;
                }
            }
        }
    }

    fn handle_normal_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('?') => self.mode = Mode::Help,
            KeyCode::Char('/') => self.mode = Mode::Search,
            KeyCode::Esc if !self.query.is_empty() => {
                self.query.clear();
                self.reset_selection();
            }
            KeyCode::Up | KeyCode::Char('k') => self.move_selection(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_selection(1),
            KeyCode::Home | KeyCode::Char('g') => self.reset_focused_selection(),
            KeyCode::End | KeyCode::Char('G') => match self.focus {
                Focus::Active => self.selected = self.visible_indices().len().saturating_sub(1),
                Focus::Recent => {
                    self.recent_selected = self.recent_visible_indices().len().saturating_sub(1)
                }
            },
            KeyCode::Tab => {
                self.focus = match self.focus {
                    Focus::Active if !self.recent.is_empty() => Focus::Recent,
                    _ => Focus::Active,
                };
                self.set_status(match self.focus {
                    Focus::Active => "Active server list focused",
                    Focus::Recent => "Launch Again focused — Enter starts the selected project",
                });
            }
            KeyCode::Char('a') => {
                self.view = self.view.toggle();
                self.reset_selection();
                self.set_status(format!("Showing {}", self.view.label()));
            }
            KeyCode::Char('r') => self.refresh_now(true),
            KeyCode::Enter => match self.focus {
                Focus::Active => self.open_selected_url(),
                Focus::Recent => self.start_selected_recent(),
            },
            KeyCode::Char('o') => self.open_focused_url(),
            KeyCode::Char('s') => self.start_selected_recent(),
            KeyCode::Char('K') => self.begin_kill_selected(),
            KeyCode::Char('p') => self.open_selected_project(),
            _ => {}
        }
    }

    fn handle_search_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.query.clear();
                self.reset_selection();
                self.mode = Mode::Normal;
            }
            KeyCode::Enter => self.mode = Mode::Normal,
            KeyCode::Backspace => {
                self.query.pop();
                self.reset_selection();
            }
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.query.push(character);
                self.reset_selection();
            }
            _ => {}
        }
    }

    fn handle_confirm_kill_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('y') | KeyCode::Enter => self.confirm_kill_selected(),
            KeyCode::Char('n') | KeyCode::Esc => {
                self.pending_kill = None;
                self.mode = Mode::Normal;
                self.set_status("Stop cancelled");
            }
            _ => {}
        }
    }

    fn move_selection(&mut self, delta: isize) {
        let count = match self.focus {
            Focus::Active => self.visible_indices().len(),
            Focus::Recent => self.recent_visible_indices().len(),
        };
        if count == 0 {
            self.reset_focused_selection();
            return;
        }
        match self.focus {
            Focus::Active => {
                self.selected = self.selected.saturating_add_signed(delta).min(count - 1)
            }
            Focus::Recent => {
                self.recent_selected = self
                    .recent_selected
                    .saturating_add_signed(delta)
                    .min(count - 1)
            }
        }
    }

    fn reset_selection(&mut self) {
        self.selected = 0;
        self.table_state.select(None);
        *self.table_state.offset_mut() = 0;
    }

    fn reset_focused_selection(&mut self) {
        match self.focus {
            Focus::Active => self.reset_selection(),
            Focus::Recent => {
                self.recent_selected = 0;
                self.recent_table_state.select(None);
                *self.recent_table_state.offset_mut() = 0;
            }
        }
    }

    fn refresh_now(&mut self, announce: bool) {
        let selected_key = self.selected_server().map(ServerInfo::key);
        match self.scanner.scan() {
            Ok(servers) => {
                self.servers = servers;
                if let Err(error) = self.history_store.record(&self.servers) {
                    self.set_status(format!("Could not save server history: {error:#}"));
                }
                self.recent = self.history_store.offline(&self.servers);
                self.recent_selected = self
                    .recent_selected
                    .min(self.recent_visible_indices().len().saturating_sub(1));
                if self.recent_visible_indices().is_empty() && self.focus == Focus::Recent {
                    self.focus = Focus::Active;
                }
                self.last_refresh = Instant::now();
                let visible = self.visible_indices();
                self.selected = selected_key
                    .and_then(|key| {
                        visible
                            .iter()
                            .position(|index| self.servers[*index].key() == key)
                    })
                    .unwrap_or_else(|| self.selected.min(visible.len().saturating_sub(1)));
                if announce {
                    self.set_status("Refreshed — detected servers saved automatically");
                }
            }
            Err(error) => {
                self.last_refresh = Instant::now();
                self.set_status(format!("Refresh failed: {error:#}"));
            }
        }
    }

    fn open_selected_url(&mut self) {
        let Some(server) = self.selected_server() else {
            self.set_status("No server selected");
            return;
        };
        let url = server.url.clone();
        match open_target(&url) {
            Ok(()) => self.set_status(format!("Opened {url}")),
            Err(error) => self.set_status(format!("Could not open browser: {error}")),
        }
    }

    fn open_focused_url(&mut self) {
        let url = match self.focus {
            Focus::Active => self.selected_server().map(|server| server.url.clone()),
            Focus::Recent => self.selected_recent().map(|server| server.url.clone()),
        };
        let Some(url) = url else {
            self.set_status("No server selected");
            return;
        };
        match open_target(&url) {
            Ok(()) => self.set_status(format!("Opened {url}")),
            Err(error) => self.set_status(format!("Could not open browser: {error}")),
        }
    }

    fn open_selected_project(&mut self) {
        let path = match self.focus {
            Focus::Active => self
                .selected_server()
                .and_then(|server| server.project_root.clone().or_else(|| server.cwd.clone())),
            Focus::Recent => self
                .selected_recent()
                .and_then(|server| server.project_root.clone()),
        };
        let Some(path) = path else {
            self.set_status("No project path is available for this listener");
            return;
        };
        match open_target(&path) {
            Ok(()) => self.set_status(format!("Opened {path}")),
            Err(error) => self.set_status(format!("Could not open project: {error}")),
        }
    }

    fn start_selected_recent(&mut self) {
        if self.focus != Focus::Recent {
            self.set_status("Press Tab to focus Launch Again first");
            return;
        }
        let Some(recent) = self.selected_recent().cloned() else {
            self.set_status("No saved project selected");
            return;
        };
        let Some(launch) = recent.launch else {
            self.set_status("No safe start command was detected for this project");
            return;
        };
        match start_launch(&launch) {
            Ok(pid) => self.set_status(format!(
                "Started {} as PID {pid}; waiting for its port",
                launch.display()
            )),
            Err(error) => self.set_status(format!("Could not start {}: {error}", launch.display())),
        }
    }

    fn begin_kill_selected(&mut self) {
        if self.focus != Focus::Active {
            self.set_status("K only stops a selected Running Now service");
            return;
        }
        let Some(server) = self.selected_server().cloned() else {
            self.set_status("No running service selected");
            return;
        };
        if server.pid <= 4 || server.pid == std::process::id() {
            self.set_status("This system service is protected and cannot be stopped");
            return;
        }
        self.pending_kill = Some(server);
        self.mode = Mode::ConfirmKill;
    }

    fn confirm_kill_selected(&mut self) {
        let Some(server) = self.pending_kill.take() else {
            self.mode = Mode::Normal;
            return;
        };
        self.mode = Mode::Normal;
        match terminate_server(&server) {
            Ok(()) => self.set_status(format!(
                "Stopped {} on port {} (PID {})",
                server.project_name, server.port, server.pid
            )),
            Err(error) => self.set_status(format!("Could not stop service: {error:#}")),
        }
        self.refresh_now(false);
    }

    fn set_status(&mut self, status: impl Into<String>) {
        self.status = status.into();
        self.status_at = Instant::now();
    }
}

fn start_launch(launch: &LaunchRecipe) -> io::Result<u32> {
    let mut command = Command::new(&launch.program);
    command
        .args(&launch.args)
        .current_dir(&launch.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        command.creation_flags(CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS);
    }
    command.spawn().map(|child| child.id())
}

pub fn open_target(target: &str) -> io::Result<()> {
    #[cfg(windows)]
    let mut command = {
        let mut command = Command::new("rundll32.exe");
        command.arg("url.dll,FileProtocolHandler").arg(target);
        command
    };
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = Command::new("open");
        command.arg(target);
        command
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = {
        let mut command = Command::new("xdg-open");
        command.arg(target);
        command
    };
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
}
