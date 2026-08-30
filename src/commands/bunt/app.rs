use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
    io,
    time::{Duration, Instant},
};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{DefaultTerminal, widgets::TableState};

use super::{
    config::ConfigStore,
    model::{KillTarget, ProcessInfo, Runtime},
    process::{ProcessScanner, TerminationOutcome},
    ui,
};

#[derive(Clone, Debug)]
pub enum Protection {
    LauncherAncestor,
    Excluded { id: String, label: String },
    Inherited { id: Option<String>, label: String },
}

impl Protection {
    pub fn label(&self) -> &str {
        match self {
            Self::LauncherAncestor => "launcher ancestor",
            Self::Excluded { label, .. } | Self::Inherited { label, .. } => label,
        }
    }

    pub fn is_excluded(&self) -> bool {
        matches!(
            self,
            Self::Excluded { .. } | Self::Inherited { id: Some(_), .. }
        )
    }

    pub fn is_safety(&self) -> bool {
        matches!(
            self,
            Self::LauncherAncestor | Self::Inherited { id: None, .. }
        )
    }

    fn rule(&self) -> Option<(&str, &str)> {
        match self {
            Self::Excluded { id, label } => Some((id, label)),
            Self::Inherited {
                id: Some(id),
                label,
                ..
            } => Some((id, label)),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ViewFilter {
    All,
    Targets,
    Protected,
}

impl ViewFilter {
    fn next(self) -> Self {
        match self {
            Self::All => Self::Targets,
            Self::Targets => Self::Protected,
            Self::Protected => Self::All,
        }
    }

    fn previous(self) -> Self {
        match self {
            Self::All => Self::Protected,
            Self::Targets => Self::All,
            Self::Protected => Self::Targets,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Targets => "targets",
            Self::Protected => "protected",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeFilter {
    All,
    One(Runtime),
}

impl RuntimeFilter {
    pub fn label(self) -> &'static str {
        match self {
            Self::All => "all runtimes",
            Self::One(runtime) => runtime.as_str(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SortKey {
    Cpu,
    Memory,
    Age,
    Name,
}

impl SortKey {
    fn next(self) -> Self {
        match self {
            Self::Cpu => Self::Memory,
            Self::Memory => Self::Age,
            Self::Age => Self::Name,
            Self::Name => Self::Cpu,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Cpu => "cpu↓",
            Self::Memory => "memory↓",
            Self::Age => "age↓",
            Self::Name => "name↑",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mode {
    Normal,
    Search,
    Confirm,
    Closing,
    Help,
}

#[derive(Clone, Debug)]
pub struct KillPreview {
    pub target: KillTarget,
    pub display: String,
}

#[derive(Clone, Debug)]
pub struct PendingKill {
    pub targets: Vec<KillPreview>,
    pub all: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClosingStage {
    Preparing,
    Graceful,
    Settling,
}

#[derive(Clone, Debug)]
pub struct CloseOperation {
    pending: PendingKill,
    eligible: Vec<KillTarget>,
    graceful: Vec<KillTarget>,
    pub stage: ClosingStage,
    pub requested: usize,
    pub eligible_count: usize,
    pub graceful_count: usize,
    pub force_requested: usize,
    pub failed: usize,
    pub skipped: usize,
    pub started_at: Instant,
    pub stage_started_at: Instant,
    pub next_action_at: Instant,
}

impl CloseOperation {
    pub(crate) fn new(pending: PendingKill) -> Self {
        let now = Instant::now();
        Self {
            requested: pending.targets.len(),
            pending,
            eligible: Vec::new(),
            graceful: Vec::new(),
            stage: ClosingStage::Preparing,
            eligible_count: 0,
            graceful_count: 0,
            force_requested: 0,
            failed: 0,
            skipped: 0,
            started_at: now,
            stage_started_at: now,
            // Let the preparing frame render before doing the first refresh.
            next_action_at: now + Duration::from_millis(30),
        }
    }

    pub fn progress(&self) -> f64 {
        match self.stage {
            ClosingStage::Preparing => 0.04,
            ClosingStage::Graceful => {
                let span = self.next_action_at.duration_since(self.stage_started_at);
                if span.is_zero() {
                    0.85
                } else {
                    (0.12
                        + 0.7
                            * (self.stage_started_at.elapsed().as_secs_f64() / span.as_secs_f64()))
                    .clamp(0.12, 0.82)
                }
            }
            ClosingStage::Settling => 0.92,
        }
    }

    pub fn stage_label(&self) -> &'static str {
        match self.stage {
            ClosingStage::Preparing => "Revalidating the selected snapshot",
            ClosingStage::Graceful => "Waiting for graceful shutdown",
            ClosingStage::Settling => "Confirming processes have exited",
        }
    }

    pub fn remaining(&self) -> Duration {
        self.next_action_at
            .saturating_duration_since(Instant::now())
    }
}

pub struct App {
    scanner: ProcessScanner,
    pub store: ConfigStore,
    pub processes: Vec<ProcessInfo>,
    launcher_ancestry: HashSet<u32>,
    pub selected: usize,
    pub view_filter: ViewFilter,
    pub runtime_filter: RuntimeFilter,
    pub sort_key: SortKey,
    pub query: String,
    pub mode: Mode,
    pub pending_kill: Option<PendingKill>,
    pub close_operation: Option<CloseOperation>,
    pub table_state: TableState,
    pub status: String,
    status_at: Instant,
    last_refresh: Instant,
    should_quit: bool,
}

impl App {
    pub fn new(mut scanner: ProcessScanner, store: ConfigStore) -> Self {
        let scan = scanner.scan();
        Self {
            scanner,
            store,
            processes: scan.processes,
            launcher_ancestry: scan.launcher_ancestry,
            selected: 0,
            view_filter: ViewFilter::All,
            runtime_filter: RuntimeFilter::All,
            sort_key: SortKey::Name,
            query: String::new(),
            mode: Mode::Normal,
            pending_kill: None,
            close_operation: None,
            table_state: TableState::default(),
            status: "Ready — press ? for help".into(),
            status_at: Instant::now(),
            last_refresh: Instant::now(),
            should_quit: false,
        }
    }

    pub fn run(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        while !self.should_quit {
            terminal.draw(|frame| ui::draw(frame, self))?;

            let refresh_interval =
                Duration::from_millis(self.store.config().behavior.refresh_ms.clamp(200, 60_000));
            let mut timeout = refresh_interval
                .saturating_sub(self.last_refresh.elapsed())
                .min(Duration::from_millis(250));
            if let Some(operation) = self.close_operation.as_ref() {
                timeout = timeout
                    .min(operation.remaining())
                    .min(Duration::from_millis(60));
            }

            if event::poll(timeout)?
                && let Event::Key(key) = event::read()?
            {
                self.handle_key(key);
            }
            if self.mode == Mode::Closing {
                self.advance_closing();
            }
            if self.last_refresh.elapsed() >= refresh_interval
                && !matches!(self.mode, Mode::Confirm | Mode::Closing)
            {
                self.refresh_now();
            }
        }
        Ok(())
    }

    pub fn protection(&self, process: &ProcessInfo) -> Option<Protection> {
        let mut current = process;
        let mut seen = HashSet::new();
        while seen.insert(current.pid) {
            if let Some(protection) = self.base_protection(current) {
                if current.pid == process.pid {
                    return Some(protection);
                }
                return Some(match protection {
                    Protection::LauncherAncestor => Protection::Inherited {
                        id: None,
                        label: format!("launcher tree via PID {}", current.pid),
                    },
                    Protection::Excluded { id, label } => Protection::Inherited {
                        id: Some(id),
                        label: format!("{label} via PID {}", current.pid),
                    },
                    Protection::Inherited { .. } => unreachable!(),
                });
            }
            let Some(parent_pid) = current.parent_pid else {
                break;
            };
            let Some(parent) = self
                .processes
                .iter()
                .find(|candidate| candidate.pid == parent_pid)
            else {
                break;
            };
            current = parent;
        }
        None
    }

    fn base_protection(&self, process: &ProcessInfo) -> Option<Protection> {
        if self.launcher_ancestry.contains(&process.pid) {
            return Some(Protection::LauncherAncestor);
        }
        self.store
            .matching_rule(&process.identity)
            .map(|rule| Protection::Excluded {
                id: rule.id.clone(),
                label: rule.name.clone(),
            })
    }

    pub fn counts(&self) -> (usize, usize, usize) {
        let mut targets = 0;
        let mut excluded = 0;
        let mut safety = 0;
        for process in &self.processes {
            match self.protection(process) {
                None => targets += 1,
                Some(protection) if protection.is_excluded() => excluded += 1,
                Some(_) => safety += 1,
            }
        }
        (targets, excluded, safety)
    }

    pub fn visible_indices(&self) -> Vec<usize> {
        let mut visible = self
            .processes
            .iter()
            .enumerate()
            .filter_map(|(index, process)| {
                let protection = self.protection(process);
                let protected = protection.is_some();
                if !matches!(self.runtime_filter, RuntimeFilter::All)
                    && !matches!(self.runtime_filter, RuntimeFilter::One(runtime) if runtime == process.runtime)
                {
                    return None;
                }
                if (self.view_filter == ViewFilter::Targets && protected)
                    || (self.view_filter == ViewFilter::Protected && !protected)
                {
                    return None;
                }
                query_score(process, protection.as_ref(), &self.query)
                    .map(|score| (index, score, protection_rank(protection.as_ref())))
            })
            .collect::<Vec<_>>();

        visible.sort_by(|left, right| {
            right
                .1
                .cmp(&left.1)
                .then_with(|| left.2.cmp(&right.2))
                .then_with(|| self.compare_processes(left.0, right.0))
        });
        visible.into_iter().map(|(index, _, _)| index).collect()
    }

    pub fn selected_process(&self) -> Option<&ProcessInfo> {
        let visible = self.visible_indices();
        visible
            .get(self.selected)
            .and_then(|index| self.processes.get(*index))
    }

    pub fn status_text(&self) -> &str {
        if self.status_at.elapsed() > Duration::from_secs(8) {
            ""
        } else {
            &self.status
        }
    }

    fn compare_processes(&self, left: usize, right: usize) -> Ordering {
        let left = &self.processes[left];
        let right = &self.processes[right];
        let stable_order = || {
            left.project_name
                .to_lowercase()
                .cmp(&right.project_name.to_lowercase())
                .then_with(|| {
                    left.workload_label
                        .to_lowercase()
                        .cmp(&right.workload_label.to_lowercase())
                })
                .then_with(|| left.pid.cmp(&right.pid))
        };

        match self.sort_key {
            // Quantized buckets prevent tiny metric fluctuations from shuffling
            // rows on every refresh. Stable identity ordering breaks ties.
            SortKey::Cpu => ((right.cpu_percent / 2.0).round() as i64)
                .cmp(&((left.cpu_percent / 2.0).round() as i64))
                .then_with(stable_order),
            SortKey::Memory => (right.memory_bytes / (16 * 1_048_576))
                .cmp(&(left.memory_bytes / (16 * 1_048_576)))
                .then_with(stable_order),
            SortKey::Age => left
                .start_time
                .cmp(&right.start_time)
                .then_with(stable_order),
            SortKey::Name => stable_order(),
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
            Mode::Confirm => self.handle_confirm_key(key),
            Mode::Closing => {
                if matches!(key.code, KeyCode::Esc | KeyCode::Char('q')) {
                    self.set_status("Shutdown is still in progress — the UI remains responsive");
                }
            }
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
            KeyCode::Home | KeyCode::Char('g') => self.reset_selection(),
            KeyCode::End | KeyCode::Char('G') => {
                self.selected = self.visible_indices().len().saturating_sub(1)
            }
            KeyCode::Tab => {
                self.view_filter = self.view_filter.next();
                self.reset_selection();
            }
            KeyCode::BackTab => {
                self.view_filter = self.view_filter.previous();
                self.reset_selection();
            }
            KeyCode::Char('1') => self.set_runtime_filter(RuntimeFilter::All),
            KeyCode::Char('2') => self.set_runtime_filter(RuntimeFilter::One(Runtime::Node)),
            KeyCode::Char('3') => self.set_runtime_filter(RuntimeFilter::One(Runtime::Bun)),
            KeyCode::Char('4') => self.set_runtime_filter(RuntimeFilter::One(Runtime::Python)),
            KeyCode::Char('s') => {
                self.sort_key = self.sort_key.next();
                self.reset_selection();
            }
            KeyCode::Char('r') => {
                self.refresh_now();
                self.set_status("Refreshed process list");
            }
            KeyCode::Char('e') => self.toggle_selected_exclude(),
            KeyCode::Char('x') => self.begin_kill_selected(),
            KeyCode::Char('K') => self.begin_kill_all(),
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

    fn handle_confirm_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('y') | KeyCode::Enter => self.start_pending_kill(),
            KeyCode::Char('n') | KeyCode::Esc => {
                self.pending_kill = None;
                self.mode = Mode::Normal;
                self.set_status("Kill cancelled");
            }
            _ => {}
        }
    }

    fn move_selection(&mut self, delta: isize) {
        let count = self.visible_indices().len();
        if count == 0 {
            self.reset_selection();
            return;
        }
        self.selected = self.selected.saturating_add_signed(delta).min(count - 1);
    }

    fn set_runtime_filter(&mut self, filter: RuntimeFilter) {
        self.runtime_filter = filter;
        self.reset_selection();
    }

    fn reset_selection(&mut self) {
        self.selected = 0;
        self.table_state.select(None);
        *self.table_state.offset_mut() = 0;
    }

    fn refresh_now(&mut self) {
        let selected_key = self.selected_process().map(ProcessInfo::key);
        let scan = self.scanner.scan();
        self.processes = scan.processes;
        self.launcher_ancestry = scan.launcher_ancestry;
        self.last_refresh = Instant::now();

        let visible = self.visible_indices();
        self.selected = selected_key
            .and_then(|key| {
                visible
                    .iter()
                    .position(|index| self.processes[*index].key() == key)
            })
            .unwrap_or_else(|| self.selected.min(visible.len().saturating_sub(1)));
    }

    fn toggle_selected_exclude(&mut self) {
        let Some(process) = self.selected_process().cloned() else {
            self.set_status("No process selected");
            return;
        };
        match self.protection(&process) {
            Some(protection) if protection.is_safety() => {
                self.set_status("This process launched bunt and is safety-protected");
            }
            Some(protection) => {
                let Some((id, label)) = protection.rule() else {
                    self.set_status("This process is safety-protected");
                    return;
                };
                let (id, label) = (id.to_owned(), label.to_owned());
                match self.store.remove_rule(&id) {
                    Ok(_) => self.set_status(format!("Removed exclusion: {label}")),
                    Err(error) => self.set_status(format!("Could not save config: {error}")),
                }
            }
            None => {
                let source = self.smart_exclusion_source(&process).clone();
                match self.store.add_workload(&source) {
                    Ok(label) if source.pid != process.pid => self.set_status(format!(
                        "Excluded process tree persistently: {label} (via PID {})",
                        source.pid
                    )),
                    Ok(label) => self.set_status(format!("Excluded persistently: {label}")),
                    Err(error) => self.set_status(format!("Could not save config: {error}")),
                }
            }
        }
        self.clamp_selection();
    }

    fn smart_exclusion_source<'a>(&'a self, process: &'a ProcessInfo) -> &'a ProcessInfo {
        let mut current = process;
        let mut seen = HashSet::new();
        while seen.insert(current.pid) {
            let Some(parent_pid) = current.parent_pid else {
                break;
            };
            let Some(parent) = self
                .processes
                .iter()
                .find(|candidate| candidate.pid == parent_pid)
            else {
                break;
            };
            if self.launcher_ancestry.contains(&parent.pid) {
                break;
            }
            current = parent;
        }
        current
    }

    fn begin_kill_selected(&mut self) {
        let Some(process) = self.selected_process() else {
            self.set_status("No process selected");
            return;
        };
        if let Some(protection) = self.protection(process) {
            self.set_status(format!("Protected by: {}", protection.label()));
            return;
        }
        let preview = preview(process);
        self.pending_kill = Some(PendingKill {
            targets: vec![preview],
            all: false,
        });
        self.mode = Mode::Confirm;
    }

    fn begin_kill_all(&mut self) {
        let mut processes = self
            .processes
            .iter()
            .filter(|process| self.protection(process).is_none())
            .collect::<Vec<_>>();
        sort_parent_first(&mut processes);
        let targets = processes.into_iter().map(preview).collect::<Vec<_>>();
        if targets.is_empty() {
            self.set_status("No non-protected processes to kill");
            return;
        }
        self.pending_kill = Some(PendingKill { targets, all: true });
        if self.store.config().behavior.confirm_kill_all {
            self.mode = Mode::Confirm;
        } else {
            self.start_pending_kill();
        }
    }

    fn start_pending_kill(&mut self) {
        let Some(pending) = self.pending_kill.take() else {
            self.mode = Mode::Normal;
            return;
        };
        let requested = pending.targets.len();
        self.close_operation = Some(CloseOperation::new(pending));
        self.mode = Mode::Closing;
        self.set_status(format!("Preparing to stop {requested} processes…"));
    }

    fn advance_closing(&mut self) {
        let Some(operation) = self.close_operation.as_ref() else {
            self.mode = Mode::Normal;
            return;
        };
        if Instant::now() < operation.next_action_at {
            return;
        }

        match operation.stage {
            ClosingStage::Preparing => self.prepare_closing(),
            ClosingStage::Graceful => self.escalate_closing(),
            ClosingStage::Settling => self.finish_closing(),
        }
    }

    fn prepare_closing(&mut self) {
        let previews = self
            .close_operation
            .as_ref()
            .map(|operation| operation.pending.targets.clone())
            .unwrap_or_default();
        self.refresh_now();

        let mut validated = Vec::new();
        let mut skipped = 0usize;
        for preview in previews {
            let Some(process) = self
                .processes
                .iter()
                .find(|process| same_target(process, &preview.target))
            else {
                skipped += 1;
                continue;
            };
            if self.protection(process).is_some() {
                skipped += 1;
                continue;
            }
            validated.push(preview.target);
        }

        let mut graceful = Vec::new();
        let mut eligible = Vec::new();
        let mut force_requested = 0usize;
        let mut failed = 0usize;
        for target in validated {
            match self.scanner.terminate(&target, false) {
                TerminationOutcome::GracefulRequested => {
                    graceful.push(target.clone());
                    eligible.push(target);
                }
                TerminationOutcome::ForceRequested => {
                    force_requested += 1;
                    eligible.push(target);
                }
                TerminationOutcome::Failed => {
                    failed += 1;
                    eligible.push(target);
                }
                TerminationOutcome::Changed => skipped += 1,
            }
        }

        let now = Instant::now();
        let (stage, next_action_at) = if graceful.is_empty() {
            (ClosingStage::Settling, now + Duration::from_millis(250))
        } else {
            let grace_period = Duration::from_millis(
                self.store
                    .config()
                    .behavior
                    .grace_period_ms
                    .clamp(100, 10_000),
            );
            (ClosingStage::Graceful, now + grace_period)
        };

        if let Some(operation) = self.close_operation.as_mut() {
            operation.eligible_count = eligible.len();
            operation.eligible = eligible;
            operation.graceful_count = graceful.len();
            operation.graceful = graceful;
            operation.force_requested = force_requested;
            operation.failed = failed;
            operation.skipped = skipped;
            operation.stage = stage;
            operation.stage_started_at = now;
            operation.next_action_at = next_action_at;
        }
        self.set_status(match stage {
            ClosingStage::Graceful => "Graceful shutdown requested — waiting before escalation",
            ClosingStage::Settling => "Termination requested — confirming exit",
            ClosingStage::Preparing => unreachable!(),
        });
    }

    fn escalate_closing(&mut self) {
        let graceful = self
            .close_operation
            .as_ref()
            .map(|operation| operation.graceful.clone())
            .unwrap_or_default();
        self.refresh_now();
        let mut force_requested = 0usize;
        let mut failed = 0usize;
        let mut became_protected = Vec::new();
        for target in graceful {
            let Some(process) = self
                .processes
                .iter()
                .find(|process| same_target(process, &target))
            else {
                continue;
            };
            if self.protection(process).is_some() {
                became_protected.push(target);
                continue;
            }
            match self.scanner.terminate(&target, true) {
                TerminationOutcome::ForceRequested => force_requested += 1,
                TerminationOutcome::Failed => failed += 1,
                TerminationOutcome::Changed => {}
                TerminationOutcome::GracefulRequested => unreachable!(),
            }
        }

        let now = Instant::now();
        if let Some(operation) = self.close_operation.as_mut() {
            operation.force_requested += force_requested;
            operation.failed += failed;
            operation.skipped += became_protected.len();
            operation
                .eligible
                .retain(|target| !became_protected.iter().any(|protected| protected == target));
            operation.eligible_count = operation.eligible.len();
            operation.stage = ClosingStage::Settling;
            operation.stage_started_at = now;
            operation.next_action_at = now + Duration::from_millis(250);
        }
        self.set_status("Grace period complete — checking force-closed processes");
    }

    fn finish_closing(&mut self) {
        self.refresh_now();
        let Some(operation) = self.close_operation.take() else {
            self.mode = Mode::Normal;
            return;
        };
        let survivors = operation
            .eligible
            .iter()
            .filter(|target| {
                self.processes
                    .iter()
                    .any(|process| same_target(process, target))
            })
            .count();
        let stopped = operation.eligible.len().saturating_sub(survivors);
        let mut message = format!("Stopped {stopped}");
        if survivors > 0 {
            message.push_str(&format!(", {survivors} still running/denied"));
        }
        if operation.skipped > 0 {
            message.push_str(&format!(
                ", {} changed or became protected",
                operation.skipped
            ));
        }
        if operation.pending.all {
            let remaining = self
                .processes
                .iter()
                .filter(|process| self.protection(process).is_none())
                .count();
            if remaining > survivors {
                message.push_str(&format!(", {remaining} new targets appeared"));
            }
        }
        self.mode = Mode::Normal;
        self.set_status(message);
    }

    fn clamp_selection(&mut self) {
        self.selected = self
            .selected
            .min(self.visible_indices().len().saturating_sub(1));
    }

    fn set_status(&mut self, message: impl Into<String>) {
        self.status = message.into();
        self.status_at = Instant::now();
    }
}

fn preview(process: &ProcessInfo) -> KillPreview {
    KillPreview {
        target: process.kill_target(),
        display: format!(
            "{}  {:>6}  {} / {}",
            process.runtime, process.pid, process.project_name, process.workload_label
        ),
    }
}

fn same_target(process: &ProcessInfo, target: &KillTarget) -> bool {
    process.pid == target.pid
        && process.start_time == target.start_time
        && process.runtime == target.runtime
        && process.identity.workload == target.workload
}

fn sort_parent_first(processes: &mut Vec<&ProcessInfo>) {
    let parents = processes
        .iter()
        .map(|process| (process.pid, process.parent_pid))
        .collect::<HashMap<_, _>>();
    processes.sort_by_key(|process| process_depth(process.pid, &parents));
}

fn process_depth(pid: u32, parents: &HashMap<u32, Option<u32>>) -> usize {
    let mut depth = 0;
    let mut current = pid;
    let mut seen = HashSet::new();
    while seen.insert(current) {
        let Some(Some(parent)) = parents.get(&current) else {
            break;
        };
        current = *parent;
        depth += 1;
    }
    depth
}

fn protection_rank(protection: Option<&Protection>) -> u8 {
    match protection {
        None => 0,
        Some(protection) if protection.is_excluded() => 1,
        Some(_) => 2,
    }
}

fn query_score(process: &ProcessInfo, protection: Option<&Protection>, query: &str) -> Option<i32> {
    if query.trim().is_empty() {
        return Some(0);
    }

    let haystack = process.searchable_text();
    let mut total = 0;
    for raw_token in query.split_whitespace() {
        let (negative, token) = raw_token
            .strip_prefix('-')
            .filter(|token| !token.is_empty())
            .map_or((false, raw_token), |token| (true, token));
        let token = token.to_lowercase();
        let score = token_score(process, protection, &haystack, &token);
        if negative {
            if score.is_some() {
                return None;
            }
        } else {
            total += score?;
        }
    }
    Some(total)
}

fn token_score(
    process: &ProcessInfo,
    protection: Option<&Protection>,
    haystack: &str,
    token: &str,
) -> Option<i32> {
    if let Some((field, value)) = token.split_once(':') {
        let field_haystack = match field {
            "rt" | "runtime" => process.runtime.as_str().to_owned(),
            "pid" => process.pid.to_string(),
            "project" | "p" => format!(
                "{} {}",
                process.project_name,
                process.project_root.as_deref().unwrap_or_default()
            )
            .to_lowercase(),
            "cwd" => process.cwd.as_deref().unwrap_or_default().to_lowercase(),
            "cmd" | "command" => process.command.to_lowercase(),
            "status" => process.status.to_lowercase(),
            "is" => return state_score(protection, value),
            _ => haystack.to_owned(),
        };
        return fuzzy_score(&field_haystack, value).map(|score| score + 40);
    }

    match token {
        "node" | "bun" | "python" if process.runtime.as_str() == token => Some(120),
        "target" if protection.is_none() => Some(120),
        "protected" if protection.is_some() => Some(120),
        "excluded" if protection.is_some_and(Protection::is_excluded) => Some(120),
        _ => fuzzy_score(haystack, token),
    }
}

fn state_score(protection: Option<&Protection>, value: &str) -> Option<i32> {
    match value {
        "target" if protection.is_none() => Some(120),
        "protected" if protection.is_some() => Some(120),
        "excluded" if protection.is_some_and(Protection::is_excluded) => Some(120),
        "ancestor" if protection.is_some_and(Protection::is_safety) => Some(120),
        _ => None,
    }
}

fn fuzzy_score(haystack: &str, needle: &str) -> Option<i32> {
    if needle.is_empty() {
        return Some(0);
    }
    if haystack == needle {
        return Some(100);
    }
    if let Some(index) = haystack.find(needle) {
        return Some(80 - i32::try_from(index.min(40)).unwrap_or(40));
    }

    let mut score = 30;
    let mut haystack = haystack.chars();
    for needle_character in needle.chars() {
        let mut skipped = 0;
        loop {
            let haystack_character = haystack.next()?;
            if haystack_character == needle_character {
                score -= skipped.min(5);
                break;
            }
            skipped += 1;
        }
    }
    Some(score.max(1))
}

#[cfg(test)]
mod tests {
    use super::super::{config::ConfigStore, model::WorkloadIdentity};
    use super::*;

    #[test]
    fn fuzzy_matching_supports_subsequences_and_negation() {
        assert!(fuzzy_score("justgains api server", "jgas").is_some());
        assert!(fuzzy_score("justgains api server", "xyz").is_none());
    }

    #[test]
    fn parent_sort_puts_supervisors_first() {
        let parents = HashMap::from([(10, None), (11, Some(10)), (12, Some(11))]);
        assert_eq!(process_depth(10, &parents), 0);
        assert_eq!(process_depth(12, &parents), 2);
    }

    #[test]
    fn exclusion_is_inherited_by_runtime_children() {
        let temp = tempfile::tempdir().unwrap();
        let store = ConfigStore::load_from(temp.path().join("config.toml")).unwrap();
        let mut app = App::new(ProcessScanner::new(), store);
        let parent = fake_process(10, None, Runtime::Bun, "command:run:dev");
        let child = fake_process(11, Some(10), Runtime::Node, "script:server.js");
        app.store.add_workload(&parent).unwrap();
        app.processes = vec![parent, child];
        app.launcher_ancestry.clear();

        let protection = app.protection(&app.processes[1]).unwrap();
        assert!(protection.is_excluded());
        assert!(protection.label().contains("via PID 10"));
        assert_eq!(app.smart_exclusion_source(&app.processes[1]).pid, 10);
    }

    #[test]
    fn metric_sort_uses_stable_buckets_instead_of_live_float_order() {
        let temp = tempfile::tempdir().unwrap();
        let store = ConfigStore::load_from(temp.path().join("config.toml")).unwrap();
        let mut app = App::new(ProcessScanner::new(), store);
        let mut beta = fake_process(11, None, Runtime::Node, "beta");
        let mut alpha = fake_process(10, None, Runtime::Node, "alpha");
        beta.cpu_percent = 1.8;
        alpha.cpu_percent = 1.1;
        app.processes = vec![beta, alpha];
        app.launcher_ancestry.clear();
        app.sort_key = SortKey::Cpu;

        assert_eq!(app.visible_indices(), vec![1, 0]);
        app.processes[0].cpu_percent = 1.2;
        app.processes[1].cpu_percent = 1.9;
        assert_eq!(app.visible_indices(), vec![1, 0]);
    }

    #[test]
    fn closing_advances_in_scheduled_stages_without_sleeping() {
        let temp = tempfile::tempdir().unwrap();
        let store = ConfigStore::load_from(temp.path().join("config.toml")).unwrap();
        let mut app = App::new(ProcessScanner::new(), store);
        app.pending_kill = Some(PendingKill {
            targets: vec![KillPreview {
                target: KillTarget {
                    pid: u32::MAX,
                    start_time: 0,
                    runtime: Runtime::Node,
                    workload: "missing".into(),
                },
                display: "controlled missing target".into(),
            }],
            all: false,
        });

        app.start_pending_kill();
        assert_eq!(app.mode, Mode::Closing);
        assert_eq!(
            app.close_operation.as_ref().unwrap().stage,
            ClosingStage::Preparing
        );

        app.close_operation.as_mut().unwrap().next_action_at = Instant::now();
        app.advance_closing();
        let operation = app.close_operation.as_ref().unwrap();
        assert_eq!(operation.stage, ClosingStage::Settling);
        assert_eq!(operation.skipped, 1);

        app.close_operation.as_mut().unwrap().next_action_at = Instant::now();
        app.advance_closing();
        assert_eq!(app.mode, Mode::Normal);
        assert!(app.close_operation.is_none());
    }

    fn fake_process(
        pid: u32,
        parent_pid: Option<u32>,
        runtime: Runtime,
        workload: &str,
    ) -> ProcessInfo {
        ProcessInfo {
            pid,
            parent_pid,
            runtime,
            process_name: runtime.to_string(),
            executable: Some(format!("/{runtime}")),
            cwd: Some("/project".into()),
            command: workload.into(),
            args: Vec::new(),
            cpu_percent: 0.0,
            memory_bytes: 0,
            virtual_memory_bytes: 0,
            disk_read_bytes: 0,
            disk_written_bytes: 0,
            start_time: 1,
            run_time: 1,
            status: "run".into(),
            project_name: "project".into(),
            project_root: Some("/project".into()),
            workload_label: workload.into(),
            identity: WorkloadIdentity {
                runtime,
                executable: Some(format!("/{runtime}")),
                anchor: Some("/project".into()),
                workload: workload.into(),
            },
        }
    }
}
