use std::{ffi::OsString, io, path::PathBuf, time::Duration};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{
    DefaultTerminal, Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, TableState, Wrap},
};

use crate::{
    console_ui,
    error::{ToolError, ToolResult},
    preferences,
};

#[derive(Clone, Debug)]
enum Kind {
    Input {
        multiple: bool,
        fallback: Option<&'static str>,
        required: bool,
    },
    Text {
        flag: &'static str,
    },
    Number {
        flag: &'static str,
        min: i64,
        max: i64,
        step: i64,
    },
    Toggle {
        flag: &'static str,
    },
    Choice {
        choices: Vec<Choice>,
    },
}

#[derive(Clone, Debug)]
struct Choice {
    label: &'static str,
    value: &'static str,
    args: &'static [&'static str],
}

#[derive(Clone, Debug)]
struct Field {
    id: &'static str,
    label: &'static str,
    help: &'static str,
    default: String,
    persistent: bool,
    kind: Kind,
}

#[derive(Clone, Debug)]
struct Tool {
    name: &'static str,
    title: &'static str,
    summary: &'static str,
    fields: Vec<Field>,
}

#[derive(Clone, Debug)]
struct Value {
    field: Field,
    value: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Mode {
    Normal,
    Edit,
    Help,
}

struct App {
    tool: Tool,
    values: Vec<Value>,
    selected: usize,
    table: TableState,
    mode: Mode,
    edit_original: String,
    store: preferences::Store,
    status: String,
    run: bool,
    quit: bool,
}

pub fn supports(command: &str) -> bool {
    spec(command).is_some()
}

pub fn run(command: &str) -> ToolResult<Option<Vec<OsString>>> {
    let tool = spec(command)
        .ok_or_else(|| ToolError::usage("just", format!("unknown tool: {command}")))?;
    let store = preferences::Store::load().map_err(|error| {
        ToolError::new(command, format!("could not load saved defaults: {error:#}"))
    })?;
    let mut app = App::new(tool, store);
    ratatui::run(|terminal| app.run(terminal))
        .map_err(|error| ToolError::new(command, format!("terminal UI failed: {error}")))?;
    if !app.run {
        return Ok(None);
    }
    app.args()
        .map(Some)
        .map_err(|message| ToolError::usage(command, message))
}

impl App {
    fn new(tool: Tool, store: preferences::Store) -> Self {
        let values = tool
            .fields
            .iter()
            .cloned()
            .map(|field| {
                let value = if field.persistent {
                    store
                        .get(tool.name, field.id)
                        .map(str::to_owned)
                        .unwrap_or_else(|| field.default.clone())
                } else {
                    field.default.clone()
                };
                let value = sanitize(&field, value);
                Value { field, value }
            })
            .collect();
        Self {
            tool,
            values,
            selected: 0,
            table: TableState::default(),
            mode: Mode::Normal,
            edit_original: String::new(),
            store,
            status: "Changes to defaults save immediately".into(),
            run: false,
            quit: false,
        }
    }

    fn run(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        while !self.quit {
            terminal.draw(|frame| draw(frame, self))?;
            if event::poll(Duration::from_millis(250))?
                && let Event::Key(key) = event::read()?
            {
                self.handle_key(key);
            }
        }
        Ok(())
    }

    fn handle_key(&mut self, key: KeyEvent) {
        if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            return;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }
        match self.mode {
            Mode::Edit => self.handle_edit(key),
            Mode::Help => {
                if matches!(
                    key.code,
                    KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q')
                ) {
                    self.mode = Mode::Normal;
                }
            }
            Mode::Normal => self.handle_normal(key),
        }
    }

    fn handle_normal(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => self.quit = true,
            KeyCode::Char('?') => self.mode = Mode::Help,
            KeyCode::Up | KeyCode::Char('k') => self.selected = self.selected.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => {
                self.selected = (self.selected + 1).min(self.values.len())
            }
            KeyCode::Home => self.selected = 0,
            KeyCode::End => self.selected = self.values.len(),
            KeyCode::Char('D') => self.reset_defaults(),
            KeyCode::Left => self.adjust(-1),
            KeyCode::Right | KeyCode::Char(' ') => self.adjust(1),
            KeyCode::Enter if self.selected == self.values.len() => self.begin_run(),
            KeyCode::Enter => self.activate_field(),
            _ => {}
        }
    }

    fn handle_edit(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.values[self.selected].value = self.edit_original.clone();
                self.mode = Mode::Normal;
                self.status = "Edit cancelled".into();
            }
            KeyCode::Enter => {
                if self.validate_selected() {
                    if self.values[self.selected].value.is_empty()
                        && !self.values[self.selected].field.default.is_empty()
                    {
                        self.values[self.selected].value =
                            self.values[self.selected].field.default.clone();
                    }
                    self.mode = Mode::Normal;
                    self.save_selected();
                }
            }
            KeyCode::Backspace => {
                self.values[self.selected].value.pop();
            }
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.values[self.selected].value.push(character);
            }
            _ => {}
        }
    }

    fn activate_field(&mut self) {
        match self.values[self.selected].field.kind {
            Kind::Toggle { .. } | Kind::Choice { .. } => self.adjust(1),
            _ => {
                self.edit_original = self.values[self.selected].value.clone();
                self.mode = Mode::Edit;
                self.status = "Editing — Enter saves, Esc cancels".into();
            }
        }
    }

    fn adjust(&mut self, direction: i64) {
        if self.selected >= self.values.len() {
            return;
        }
        let entry = &mut self.values[self.selected];
        match &entry.field.kind {
            Kind::Toggle { .. } => {
                entry.value = if entry.value == "true" {
                    "false"
                } else {
                    "true"
                }
                .into()
            }
            Kind::Choice { choices } => {
                let current = choices
                    .iter()
                    .position(|choice| choice.value == entry.value)
                    .unwrap_or(0) as i64;
                let index = (current + direction).rem_euclid(choices.len() as i64) as usize;
                entry.value = choices[index].value.into();
            }
            Kind::Number { min, max, step, .. } => {
                let current = entry.value.parse::<i64>().unwrap_or(*min);
                entry.value = (current + direction * *step).clamp(*min, *max).to_string();
            }
            Kind::Input { .. } | Kind::Text { .. } => return,
        }
        self.save_selected();
    }

    fn validate_selected(&mut self) -> bool {
        let entry = &self.values[self.selected];
        if let Kind::Number { min, max, .. } = entry.field.kind
            && entry
                .value
                .parse::<i64>()
                .ok()
                .is_none_or(|value| value < min || value > max)
        {
            self.status = format!("{} must be from {min} to {max}", entry.field.label);
            return false;
        }
        true
    }

    fn save_selected(&mut self) {
        let entry = &self.values[self.selected];
        if !entry.field.persistent {
            self.status = "One-run value changed; it will reset next time".into();
            return;
        }
        let saved = (entry.value != entry.field.default).then_some(entry.value.as_str());
        match self.store.set(self.tool.name, entry.field.id, saved) {
            Ok(()) => {
                self.status = if saved.is_some() {
                    "Default saved"
                } else {
                    "Built-in default restored"
                }
                .into()
            }
            Err(error) => self.status = format!("Could not save default: {error:#}"),
        }
    }

    fn reset_defaults(&mut self) {
        match self.store.reset_tool(self.tool.name) {
            Ok(()) => {
                for value in &mut self.values {
                    if value.field.persistent {
                        value.value = value.field.default.clone();
                    }
                }
                self.status = "All saved defaults for this tool were reset".into();
            }
            Err(error) => self.status = format!("Could not reset defaults: {error:#}"),
        }
    }

    fn begin_run(&mut self) {
        match self.args() {
            Ok(_) => {
                self.run = true;
                self.quit = true;
            }
            Err(message) => self.status = message,
        }
    }

    fn args(&self) -> Result<Vec<OsString>, String> {
        self.validate_tool()?;
        let mut args = Vec::new();
        for entry in &self.values {
            if self.skip_in_command(entry.field.id) {
                continue;
            }
            match &entry.field.kind {
                Kind::Input {
                    multiple,
                    fallback,
                    required,
                } => {
                    let mut values = if *multiple {
                        split_inputs(&entry.value)
                    } else if entry.value.trim().is_empty() {
                        Vec::new()
                    } else {
                        vec![entry.value.trim().to_owned()]
                    };
                    if values.is_empty()
                        && let Some(fallback) = fallback
                    {
                        values.push((*fallback).into());
                    }
                    if values.is_empty() && *required {
                        return Err(format!("{} is required", entry.field.label));
                    }
                    if values.is_empty() && self.tool.name == "justrmbg" && !self.enabled("check") {
                        return Err("Image or folder is required unless Runtime check is on".into());
                    }
                    if !multiple && values.len() > 1 {
                        return Err(format!("{} accepts one value", entry.field.label));
                    }
                    args.extend(values.into_iter().map(OsString::from));
                }
                Kind::Text { flag } | Kind::Number { flag, .. } => {
                    if !entry.value.is_empty() && entry.value != entry.field.default {
                        args.push(OsString::from(*flag));
                        args.push(OsString::from(&entry.value));
                    }
                }
                Kind::Toggle { flag } => {
                    if entry.value == "true" {
                        args.push(OsString::from(*flag));
                    }
                }
                Kind::Choice { choices } => {
                    if let Some(choice) = choices.iter().find(|choice| choice.value == entry.value)
                    {
                        args.extend(choice.args.iter().map(OsString::from));
                    }
                }
            }
        }
        Ok(args)
    }

    fn validate_tool(&self) -> Result<(), String> {
        let integer = |id: &str, label: &str, min: u32, max: u32| -> Result<(), String> {
            let value = self.value(id);
            if value.is_empty() {
                return Ok(());
            }
            value
                .parse::<u32>()
                .ok()
                .filter(|value| (*value >= min) && (*value <= max))
                .map(|_| ())
                .ok_or_else(|| format!("{label} must be from {min} to {max}"))
        };
        match self.tool.name {
            "justresize" => {
                integer("width", "Width", 1, 65_535)?;
                integer("height", "Height", 1, 65_535)?;
                if self.enabled("crop")
                    && (self.value("width").is_empty() || self.value("height").is_empty())
                {
                    return Err("Center crop requires both Width and Height".into());
                }
            }
            "justjpg" => {
                let value = self.value("background");
                if !matches!(value.to_ascii_lowercase().as_str(), "white" | "black") {
                    let hex = value.strip_prefix('#').unwrap_or(value);
                    if hex.len() != 6 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                        return Err(
                            "Alpha background must be white, black, or six-digit RRGGBB".into()
                        );
                    }
                }
            }
            "justpng" => {
                let parts = self.value("quality").split_once('-');
                let valid = parts.is_some_and(|(low, high)| {
                    let low = low.parse::<u8>().ok();
                    let high = high.parse::<u8>().ok();
                    matches!((low, high), (Some(low), Some(high)) if low <= high && high <= 100)
                });
                if !valid {
                    return Err("Quality range must look like 65-90 and stay within 0-100".into());
                }
            }
            "justaudio" | "justvideo" => {
                let id = if self.tool.name == "justaudio" {
                    "bitrate"
                } else {
                    "audio_bitrate"
                };
                let value = self.value(id);
                let body = value.strip_suffix(['k', 'K', 'm', 'M']).unwrap_or(value);
                if body.parse::<f64>().is_err() {
                    return Err("Audio bitrate must look like 160k".into());
                }
            }
            "justport" => {
                for port in split_inputs(self.value("input")) {
                    if port.parse::<u16>().ok().is_none_or(|port| port == 0) {
                        return Err(format!("Port must be from 1 to 65535: {port}"));
                    }
                }
            }
            "justpdf" if self.value("operation") == "extract" && self.value("pages").is_empty() => {
                return Err("Extract requires a Page range".into());
            }
            "justqr" => {
                for (id, label) in [("dark", "Foreground"), ("light", "Background")] {
                    let value = self.value(id).strip_prefix('#').unwrap_or(self.value(id));
                    if !matches!(value.len(), 6 | 8)
                        || !value.bytes().all(|byte| byte.is_ascii_hexdigit())
                    {
                        return Err(format!("{label} must be a six- or eight-digit hex color"));
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn enabled(&self, id: &str) -> bool {
        self.values
            .iter()
            .any(|value| value.field.id == id && value.value == "true")
    }

    fn value(&self, id: &str) -> &str {
        self.values
            .iter()
            .find(|value| value.field.id == id)
            .map_or("", |value| value.value.as_str())
    }

    fn skip_in_command(&self, id: &str) -> bool {
        if id == "replace" && !self.value("output").is_empty() {
            return true;
        }
        match self.tool.name {
            "justresize" if id == "max" => {
                !self.value("width").is_empty() || !self.value("height").is_empty()
            }
            "justjson" if !self.value("get").is_empty() => {
                matches!(id, "check" | "output" | "dry_run")
            }
            "justjson" if self.enabled("minify") => id == "indent",
            "justqr" if self.value("format") == "terminal" => id == "output",
            "justport" if self.enabled("kill") => id == "json",
            "justrmbg" if self.enabled("check") => matches!(id, "input" | "output" | "model"),
            "justcommit" if !self.enabled("repair") => id == "repair_agent",
            _ => false,
        }
    }

    fn command(&self) -> String {
        match self.args() {
            Ok(args) => std::iter::once(self.tool.name.to_owned())
                .chain(args.iter().map(|arg| quote(&arg.to_string_lossy())))
                .collect::<Vec<_>>()
                .join(" "),
            Err(_) => format!("{}  <complete required fields>", self.tool.name),
        }
    }

    fn destination(&self, pattern: &str) -> String {
        let output = self.value("output").trim();
        if output.is_empty() {
            return format!("beside each source as {pattern}");
        }
        let path = PathBuf::from(output);
        let absolute = if path.is_absolute() {
            path
        } else {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(path)
        };
        format!("{}/{}", absolute.display(), pattern)
    }

    fn output_policy(&self) -> (String, String) {
        let separate_output = !self.value("output").trim().is_empty();
        match self.tool.name {
            "justpng" => (
                format!("Output: {}", self.destination("<same-name>.png")),
                if separate_output {
                    "Overwrite: sources kept; existing destination files replaced atomically."
                } else {
                    "Overwrite: source PNG only replaced atomically when the result is smaller."
                }
                .into(),
            ),
            "justwebp" => (
                format!("Output: {}", self.destination("<name>.webp")),
                if separate_output {
                    "Overwrite: sources kept; existing destination files replaced atomically."
                } else {
                    "Overwrite: smaller WebP replaces its target, then the original is removed."
                }
                .into(),
            ),
            "justavif" => (
                format!("Output: {}", self.destination("<name>.avif")),
                if separate_output {
                    "Overwrite: sources kept; existing destination files replaced atomically."
                } else {
                    "Overwrite: smaller AVIF replaces its target, then the original is removed."
                }
                .into(),
            ),
            "justjpg" => (
                format!(
                    "Output: {}",
                    self.destination(if self.enabled("replace") || separate_output {
                        "<name>.jpg"
                    } else {
                        "<name>-optimized.jpg"
                    })
                ),
                if separate_output || !self.enabled("replace") {
                    "Overwrite: source kept; an existing destination is replaced atomically."
                } else {
                    "Overwrite: JPEG replaced, or converted source removed, after safe install."
                }
                .into(),
            ),
            "justoptimize" => (
                format!(
                    "Output: {}",
                    self.destination(if self.enabled("replace") || separate_output {
                        "<name>.<best>"
                    } else {
                        "<name>-optimized.<best>"
                    })
                ),
                if separate_output || !self.enabled("replace") {
                    "Overwrite: source kept; existing destinations require confirmation."
                } else {
                    "Overwrite: source removed/replaced only after the smallest result is safe."
                }
                .into(),
            ),
            "justresize" => (
                format!(
                    "Output: {}",
                    self.destination(if self.enabled("replace") || separate_output {
                        "<same-name>.<same-format>"
                    } else {
                        "<name>-resized.<same-format>"
                    })
                ),
                if self.enabled("replace") && !separate_output {
                    "Overwrite: source replaced atomically; folder runs still confirm."
                } else {
                    "Overwrite: source kept; existing destination replaced atomically."
                }
                .into(),
            ),
            "justcrop" => (
                format!(
                    "Output: {}",
                    self.destination(if self.enabled("replace") || separate_output {
                        "<same-name>.<same-format>"
                    } else {
                        "<name>-cropped.<same-format>"
                    })
                ),
                if self.enabled("replace") && !separate_output {
                    "Overwrite: source replaced atomically; folder runs still confirm."
                } else {
                    "Overwrite: source kept; existing destination replaced atomically."
                }
                .into(),
            ),
            "justrmbg" if !self.enabled("check") => {
                let destination = if self.value("output").trim().is_empty() {
                    "beside each input as <name>-nobg.png".into()
                } else {
                    let path = PathBuf::from(self.value("output").trim());
                    let absolute = if path.is_absolute() {
                        path
                    } else {
                        std::env::current_dir()
                            .unwrap_or_else(|_| PathBuf::from("."))
                            .join(path)
                    };
                    format!(
                        "{} (exact file for one input; directory for a batch)",
                        absolute.display()
                    )
                };
                (
                    format!("Output: {destination}"),
                    "Overwrite: input is always kept; an existing PNG is replaced atomically."
                        .into(),
                )
            }
            "justvideo" => (
                format!(
                    "Output: {}",
                    self.destination(if self.enabled("replace") {
                        "<name>.mp4"
                    } else {
                        "<name>-web.mp4"
                    })
                ),
                if self.enabled("replace") && !separate_output {
                    "Overwrite: source replaced/removed only after the MP4 is safely installed."
                } else {
                    "Overwrite: source kept; existing destination replaced atomically."
                }
                .into(),
            ),
            "justaudio" | "justmp3" | "justwav" => {
                let extension = match self.tool.name {
                    "justaudio" => "m4a",
                    "justmp3" => "mp3",
                    _ => "wav",
                };
                (
                    format!(
                        "Output: {}",
                        self.destination(&format!("<name>.{extension}"))
                    ),
                    if self.enabled("replace") && !separate_output {
                        "Overwrite: source replaced/removed only after safe output installation."
                    } else {
                        "Overwrite: source kept; existing destination replaced atomically."
                    }
                    .into(),
                )
            }
            _ => (String::new(), String::new()),
        }
    }
}

fn sanitize(field: &Field, value: String) -> String {
    match &field.kind {
        Kind::Toggle { .. } if !matches!(value.as_str(), "true" | "false") => field.default.clone(),
        Kind::Choice { choices } if !choices.iter().any(|choice| choice.value == value) => {
            field.default.clone()
        }
        Kind::Number { min, max, .. }
            if value
                .parse::<i64>()
                .ok()
                .is_none_or(|number| number < *min || number > *max) =>
        {
            field.default.clone()
        }
        _ => value,
    }
}

fn draw(frame: &mut Frame<'_>, app: &mut App) {
    let [header, fields, details, footer] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(6),
        Constraint::Length(7),
        Constraint::Length(4),
    ])
    .areas(frame.area());
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(format!(" {} ", app.tool.summary), console_ui::GOOD),
            Span::styled("  saved defaults: automatic ", console_ui::MUTED),
        ]))
        .block(Block::default().borders(Borders::ALL).title(Span::styled(
            format!(" {} ", app.tool.title),
            Style::default().bold(),
        ))),
        header,
    );

    let rows = app
        .values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let editing = app.mode == Mode::Edit && index == app.selected;
            let shown = display_value(value, editing);
            let scope = if value.field.persistent {
                "saved"
            } else {
                "this run"
            };
            Row::new(vec![
                Cell::from(value.field.label),
                Cell::from(shown).style(console_ui::bold(console_ui::ACCENT)),
                Cell::from(scope).style(console_ui::MUTED),
            ])
        })
        .chain(std::iter::once(Row::new(vec![
            Cell::from("Run"),
            Cell::from("Enter to continue").style(console_ui::bold(console_ui::GOOD)),
            Cell::from("action").style(console_ui::MUTED),
        ])))
        .collect::<Vec<_>>();
    let table = Table::new(
        rows,
        [
            Constraint::Length(21),
            Constraint::Min(24),
            Constraint::Length(10),
        ],
    )
    .header(
        Row::new(["Setting", "Value", "Scope"])
            .style(Style::default().fg(Color::White).bold())
            .bottom_margin(1),
    )
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(console_ui::ACCENT)
            .title(" Configuration "),
    )
    .row_highlight_style(console_ui::selected())
    .highlight_symbol("› ");
    app.table.select(Some(app.selected));
    frame.render_stateful_widget(table, fields, &mut app.table);

    let detail = if app.selected == app.values.len() {
        "Restore the terminal, then run the command shown below. Normal safety confirmations still apply."
    } else {
        app.values[app.selected].field.help
    };
    let (output, overwrite) = app.output_policy();
    let mut details_lines = vec![Line::raw(detail)];
    if !output.is_empty() {
        details_lines.push(Line::styled(output, console_ui::GOOD));
        details_lines.push(Line::styled(overwrite, console_ui::SECONDARY));
    }
    frame.render_widget(
        Paragraph::new(details_lines)
            .wrap(Wrap { trim: true })
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Details and outcome "),
            ),
        details,
    );

    let keys = match app.mode {
        Mode::Edit => "type value  Enter save  Esc cancel",
        Mode::Help => "? / q / Esc close help",
        Mode::Normal => {
            "↑↓ move  ←→/Space change  Enter edit/run  D reset saved defaults  ? help  q quit"
        }
    };
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(keys, console_ui::MUTED)),
            Line::from(Span::styled(&app.status, console_ui::ACCENT)),
            Line::from(vec![
                Span::styled("Headless: ", console_ui::MUTED),
                Span::styled(app.command(), console_ui::bold(console_ui::SECONDARY)),
            ]),
        ])
        .wrap(Wrap { trim: false }),
        footer,
    );
    if app.mode == Mode::Help {
        draw_help(frame, app);
    }
}

fn display_value(value: &Value, editing: bool) -> String {
    let cursor = if editing { "▌" } else { "" };
    match &value.field.kind {
        Kind::Input {
            fallback: Some(fallback),
            ..
        } if value.value.is_empty() => format!("{fallback}  (current folder)"),
        Kind::Input { required: true, .. } if value.value.is_empty() => {
            "<required — Enter to edit>".into()
        }
        Kind::Text { .. } if value.value.is_empty() => format!("<not set>{cursor}"),
        Kind::Toggle { .. } => if value.value == "true" { "On" } else { "Off" }.into(),
        Kind::Choice { choices } => choices
            .iter()
            .find(|choice| choice.value == value.value)
            .map_or_else(|| value.value.clone(), |choice| choice.label.into()),
        _ => format!("{}{cursor}", value.value),
    }
}

fn draw_help(frame: &mut Frame<'_>, app: &App) {
    let area = centered(frame.area(), 78, 18);
    frame.render_widget(Clear, area);
    let lines = vec![
        Line::styled(
            "A launcher for the same command-line tool",
            console_ui::bold(console_ui::ACCENT),
        ),
        Line::raw(""),
        Line::raw("  ↑↓ / j k      Move through settings"),
        Line::raw("  ←→ / Space    Toggle or cycle a setting"),
        Line::raw("  Enter         Edit text/numbers, or run from the Run row"),
        Line::raw("  D             Reset every saved default for this tool"),
        Line::raw(""),
        Line::styled("Saving", console_ui::bold(console_ui::ACCENT)),
        Line::raw("  Rows marked saved are written immediately when changed."),
        Line::raw("  Inputs and one-run actions never become defaults."),
        Line::raw("  Confirmation bypasses and credentials are never stored."),
        Line::raw(""),
        Line::styled(
            format!("  Defaults file: {}", app.store.file_path().display()),
            console_ui::MUTED,
        ),
        Line::styled(
            "  The bottom line is the exact non-UI command.",
            console_ui::MUTED,
        ),
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(console_ui::ACCENT)
                    .title(" JustTools launcher help "),
            )
            .wrap(Wrap { trim: true }),
        area,
    );
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width.saturating_sub(2)).max(1);
    let height = height.min(area.height.saturating_sub(2)).max(1);
    let [vertical] = Layout::vertical([Constraint::Length(height)])
        .flex(ratatui::layout::Flex::Center)
        .areas(area);
    let [center] = Layout::horizontal([Constraint::Length(width)])
        .flex(ratatui::layout::Flex::Center)
        .areas(vertical);
    center
}

fn split_inputs(value: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    for character in value.chars() {
        match (quote, character) {
            (Some(active), value) if value == active => quote = None,
            (None, '\'' | '"') => quote = Some(character),
            (None, ';') => {
                if !current.trim().is_empty() {
                    result.push(current.trim().to_owned());
                }
                current.clear();
            }
            _ => current.push(character),
        }
    }
    if !current.trim().is_empty() {
        result.push(current.trim().to_owned());
    }
    result
}

fn quote(value: &str) -> String {
    if !value.is_empty()
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "-._/:\\@".contains(character))
    {
        value.into()
    } else if cfg!(windows) {
        format!("'{}'", value.replace('\'', "''"))
    } else {
        format!("'{}'", value.replace('\'', "'\"'\"'"))
    }
}

fn choice(label: &'static str, value: &'static str, args: &'static [&'static str]) -> Choice {
    Choice { label, value, args }
}
fn input(
    label: &'static str,
    help: &'static str,
    multiple: bool,
    fallback: Option<&'static str>,
    required: bool,
) -> Field {
    Field {
        id: "input",
        label,
        help,
        default: String::new(),
        persistent: false,
        kind: Kind::Input {
            multiple,
            fallback,
            required,
        },
    }
}
fn text(
    id: &'static str,
    label: &'static str,
    help: &'static str,
    default: &str,
    flag: &'static str,
    persistent: bool,
) -> Field {
    Field {
        id,
        label,
        help,
        default: default.into(),
        persistent,
        kind: Kind::Text { flag },
    }
}
#[allow(clippy::too_many_arguments)] // Keeps each declarative field definition readable in the schema below.
fn number(
    id: &'static str,
    label: &'static str,
    help: &'static str,
    default: i64,
    flag: &'static str,
    min: i64,
    max: i64,
    step: i64,
) -> Field {
    Field {
        id,
        label,
        help,
        default: default.to_string(),
        persistent: true,
        kind: Kind::Number {
            flag,
            min,
            max,
            step,
        },
    }
}
fn toggle(
    id: &'static str,
    label: &'static str,
    help: &'static str,
    flag: &'static str,
    persistent: bool,
) -> Field {
    toggle_default(id, label, help, flag, persistent, false)
}

fn toggle_default(
    id: &'static str,
    label: &'static str,
    help: &'static str,
    flag: &'static str,
    persistent: bool,
    default: bool,
) -> Field {
    Field {
        id,
        label,
        help,
        default: default.to_string(),
        persistent,
        kind: Kind::Toggle { flag },
    }
}
fn choices(
    id: &'static str,
    label: &'static str,
    help: &'static str,
    default: &'static str,
    options: Vec<Choice>,
) -> Field {
    Field {
        id,
        label,
        help,
        default: default.into(),
        persistent: true,
        kind: Kind::Choice { choices: options },
    }
}
fn jobs(max: usize) -> i64 {
    std::thread::available_parallelism()
        .map_or(1, usize::from)
        .min(max) as i64
}

fn common_file_fields(fields: &mut Vec<Field>, job_default: i64, output_help: &'static str) {
    fields.push(text(
        "output",
        "Output folder",
        output_help,
        "",
        "--output",
        true,
    ));
    fields.push(number(
        "jobs",
        "Parallel jobs",
        "Number of files processed in parallel.",
        job_default,
        "--jobs",
        1,
        256,
        1,
    ));
    fields.push(toggle(
        "recursive",
        "Recursive",
        "Include files in nested folders.",
        "--recursive",
        true,
    ));
    fields.push(toggle(
        "dry_run",
        "Dry run",
        "Preview this run without writing files. This one-run safety action is not saved.",
        "--dry-run",
        false,
    ));
}

fn media_spec(name: &'static str) -> Tool {
    let (title, summary, mut fields, job_default) = match name {
        "justpng" => (
            "JustPNG",
            "Optimize PNG files",
            vec![
                input(
                    "Input files / folders",
                    "Use semicolons between multiple inputs. Blank processes the current folder.",
                    true,
                    Some("."),
                    false,
                ),
                text(
                    "quality",
                    "Quality range",
                    "pngquant minimum-maximum quality range.",
                    "65-90",
                    "--quality",
                    true,
                ),
                number(
                    "speed",
                    "Encoder speed",
                    "1 is slowest/best; 11 is fastest.",
                    3,
                    "--speed",
                    1,
                    11,
                    1,
                ),
            ],
            jobs(8),
        ),
        "justwebp" => (
            "JustWebP",
            "Create compact WebP images",
            vec![
                input(
                    "Input files / folders",
                    "Use semicolons between multiple inputs. Blank processes the current folder.",
                    true,
                    Some("."),
                    false,
                ),
                number(
                    "quality",
                    "Quality",
                    "Lossy WebP visual quality from 0 to 100.",
                    82,
                    "--quality",
                    0,
                    100,
                    1,
                ),
                number(
                    "method",
                    "Method",
                    "Compression effort from 0 to 6.",
                    5,
                    "--method",
                    0,
                    6,
                    1,
                ),
                toggle(
                    "include_target",
                    "Re-encode WebP",
                    "Include existing WebP inputs.",
                    "--include-webp",
                    true,
                ),
            ],
            jobs(4),
        ),
        "justavif" => (
            "JustAVIF",
            "Create compact AVIF images",
            vec![
                input(
                    "Input files / folders",
                    "Use semicolons between multiple inputs. Blank processes the current folder.",
                    true,
                    Some("."),
                    false,
                ),
                number(
                    "quality",
                    "Quality",
                    "AV1 visual quality from 0 to 100.",
                    60,
                    "--quality",
                    0,
                    100,
                    1,
                ),
                number(
                    "speed",
                    "Encoder speed",
                    "0 is slowest/best; 8 is fastest.",
                    6,
                    "--speed",
                    0,
                    8,
                    1,
                ),
                toggle(
                    "include_target",
                    "Re-encode AVIF",
                    "Include existing AVIF inputs.",
                    "--include-avif",
                    true,
                ),
            ],
            jobs(2),
        ),
        "justvideo" => (
            "JustVideo",
            "Create streaming-ready video",
            vec![
                input(
                    "Input files / folders",
                    "Use semicolons between multiple inputs. Blank processes the current folder.",
                    true,
                    Some("."),
                    false,
                ),
                number(
                    "crf",
                    "Video CRF",
                    "H.264 quality: lower is larger and higher quality.",
                    28,
                    "--crf",
                    0,
                    51,
                    1,
                ),
                text(
                    "preset",
                    "x264 preset",
                    "Encoder preset such as fast, medium, slow, or veryslow.",
                    "medium",
                    "--preset",
                    true,
                ),
                text(
                    "audio_bitrate",
                    "Audio bitrate",
                    "AAC bitrate such as 128k or 192k.",
                    "128k",
                    "--audio-bitrate",
                    true,
                ),
                toggle(
                    "replace",
                    "Replace sources",
                    "Replace source files after a safe encode. Saved, but normal folder confirmation still applies.",
                    "--replace",
                    true,
                ),
            ],
            jobs(2),
        ),
        "justaudio" => (
            "JustAudio",
            "Create compact AAC audio",
            vec![
                input(
                    "Input files / folders",
                    "Use semicolons between multiple inputs. Blank processes the current folder.",
                    true,
                    Some("."),
                    false,
                ),
                text(
                    "bitrate",
                    "AAC bitrate",
                    "AAC bitrate such as 160k or 256k.",
                    "160k",
                    "--bitrate",
                    true,
                ),
                number(
                    "sample_rate",
                    "Sample rate",
                    "Output sample rate in Hz.",
                    48_000,
                    "--sample-rate",
                    8_000,
                    192_000,
                    1_000,
                ),
                toggle(
                    "reencode",
                    "Re-encode AAC",
                    "Include files already in the target format.",
                    "--reencode",
                    true,
                ),
                toggle(
                    "replace",
                    "Remove sources",
                    "Remove each source only after output is safely installed.",
                    "--replace",
                    true,
                ),
            ],
            jobs(2),
        ),
        "justmp3" => (
            "JustMP3",
            "Create high-quality MP3 audio",
            vec![
                input(
                    "Input files / folders",
                    "Use semicolons between multiple inputs. Blank processes the current folder.",
                    true,
                    Some("."),
                    false,
                ),
                number(
                    "quality",
                    "VBR quality",
                    "LAME VBR quality: 0 is best and 9 is smallest.",
                    2,
                    "--quality",
                    0,
                    9,
                    1,
                ),
                number(
                    "sample_rate",
                    "Sample rate",
                    "Output sample rate in Hz.",
                    48_000,
                    "--sample-rate",
                    8_000,
                    192_000,
                    1_000,
                ),
                toggle(
                    "reencode",
                    "Re-encode MP3",
                    "Include files already in the target format.",
                    "--reencode",
                    true,
                ),
                toggle(
                    "replace",
                    "Remove sources",
                    "Remove each source only after output is safely installed.",
                    "--replace",
                    true,
                ),
            ],
            jobs(2),
        ),
        "justwav" => (
            "JustWAV",
            "Create editing-ready WAV audio",
            vec![
                input(
                    "Input files / folders",
                    "Use semicolons between multiple inputs. Blank processes the current folder.",
                    true,
                    Some("."),
                    false,
                ),
                choices(
                    "bits",
                    "PCM depth",
                    "Output PCM bit depth.",
                    "16",
                    vec![
                        choice("16-bit", "16", &[]),
                        choice("24-bit", "24", &["--bits", "24"]),
                    ],
                ),
                number(
                    "sample_rate",
                    "Sample rate",
                    "Output sample rate in Hz.",
                    48_000,
                    "--sample-rate",
                    8_000,
                    192_000,
                    1_000,
                ),
                toggle(
                    "reencode",
                    "Re-encode WAV",
                    "Include files already in the target format.",
                    "--reencode",
                    true,
                ),
                toggle(
                    "replace",
                    "Remove sources",
                    "Remove each source only after output is safely installed.",
                    "--replace",
                    true,
                ),
            ],
            jobs(2),
        ),
        _ => unreachable!(),
    };
    common_file_fields(
        &mut fields,
        job_default,
        "Optional destination folder. Blank uses the tool's beside-source behavior.",
    );
    Tool {
        name,
        title,
        summary,
        fields,
    }
}

fn spec(name: &str) -> Option<Tool> {
    if matches!(
        name,
        "justaudio" | "justavif" | "justmp3" | "justpng" | "justvideo" | "justwav" | "justwebp"
    ) {
        return Some(media_spec(match name {
            "justaudio" => "justaudio",
            "justavif" => "justavif",
            "justmp3" => "justmp3",
            "justpng" => "justpng",
            "justvideo" => "justvideo",
            "justwav" => "justwav",
            _ => "justwebp",
        }));
    }
    Some(match name {
        "justcrop" => {
            let mut fields = vec![
                input(
                    "Input files / folders",
                    "Use semicolons between multiple inputs. Blank processes the current folder.",
                    true,
                    Some("."),
                    false,
                ),
                choices(
                    "bounds",
                    "Bounds mode",
                    "Individual makes each image tight; shared keeps frame sequences aligned.",
                    "individual",
                    vec![
                        choice("Individual per image", "individual", &[]),
                        choice("Shared per folder", "shared", &["--shared-bounds"]),
                    ],
                ),
                number(
                    "threshold",
                    "Alpha threshold",
                    "Ignore alpha values at or below this value.",
                    0,
                    "--threshold",
                    0,
                    254,
                    1,
                ),
                number(
                    "padding",
                    "Padding",
                    "Transparent pixels to retain around visible bounds.",
                    0,
                    "--padding",
                    0,
                    65_535,
                    1,
                ),
                toggle(
                    "replace",
                    "Replace sources",
                    "Atomically replace source images. Saved, but normal folder confirmation still applies.",
                    "--replace",
                    true,
                ),
            ];
            common_file_fields(
                &mut fields,
                jobs(8),
                "Optional destination folder; blank writes named copies beside sources.",
            );
            Tool {
                name: "justcrop",
                title: "JustCrop",
                summary: "Trim transparent image borders",
                fields,
            }
        }
        "justjpg" => {
            let mut fields = vec![
                input(
                    "Input files / folders",
                    "Use semicolons between multiple inputs. Blank processes the current folder.",
                    true,
                    Some("."),
                    false,
                ),
                number(
                    "quality",
                    "JPEG quality",
                    "JPEG visual quality from 1 to 100.",
                    85,
                    "--quality",
                    1,
                    100,
                    1,
                ),
                text(
                    "background",
                    "Alpha background",
                    "white, black, or a six-digit RRGGBB color.",
                    "white",
                    "--background",
                    true,
                ),
                toggle(
                    "baseline",
                    "Baseline JPEG",
                    "Use baseline rather than progressive encoding.",
                    "--baseline",
                    true,
                ),
                toggle(
                    "replace",
                    "Replace sources",
                    "Replace JPEGs or remove converted sources after safe output.",
                    "--replace",
                    true,
                ),
            ];
            common_file_fields(
                &mut fields,
                jobs(4),
                "Optional destination folder; blank writes optimized files beside sources.",
            );
            Tool {
                name: "justjpg",
                title: "JustJPG",
                summary: "Create web-ready JPEG images",
                fields,
            }
        }
        "justoptimize" => {
            let mut fields = vec![
                input(
                    "Input files / folders",
                    "Use semicolons between inputs. Blank evaluates the current folder.",
                    true,
                    Some("."),
                    false,
                ),
                number(
                    "quality",
                    "Web quality",
                    "Visual quality used for the WebP and progressive JPEG candidates.",
                    82,
                    "--quality",
                    1,
                    100,
                    1,
                ),
                toggle(
                    "replace",
                    "Replace sources",
                    "Remove or replace a source only after the smallest candidate is installed.",
                    "--replace",
                    true,
                ),
            ];
            common_file_fields(
                &mut fields,
                jobs(4),
                "Optional destination folder. Blank writes <name>-optimized.<best> beside sources.",
            );
            Tool {
                name: "justoptimize",
                title: "JustOptimize",
                summary: "Choose the smallest web-ready image format",
                fields,
            }
        }
        "justresize" => {
            let mut fields = vec![
                input(
                    "Input files / folders",
                    "Use semicolons between multiple inputs. Blank processes the current folder.",
                    true,
                    Some("."),
                    false,
                ),
                number(
                    "max",
                    "Maximum size",
                    "Fit inside this square. Leave Width and Height blank when changing this.",
                    1920,
                    "--max",
                    1,
                    65_535,
                    10,
                ),
                text(
                    "width",
                    "Width",
                    "Optional target width. The built-in 1920 maximum is omitted automatically when Width is set.",
                    "",
                    "--width",
                    true,
                ),
                text(
                    "height",
                    "Height",
                    "Optional target height. Set both Width and Height to use center crop.",
                    "",
                    "--height",
                    true,
                ),
                toggle(
                    "crop",
                    "Center crop",
                    "Requires both Width and Height.",
                    "--crop",
                    true,
                ),
                toggle(
                    "upscale",
                    "Allow upscale",
                    "Permit enlargement of smaller images.",
                    "--upscale",
                    true,
                ),
                number(
                    "quality",
                    "JPEG quality",
                    "JPEG quality used when resizing JPEG inputs.",
                    85,
                    "--quality",
                    1,
                    100,
                    1,
                ),
                toggle(
                    "replace",
                    "Replace sources",
                    "Atomically replace source images. Saved, but normal folder confirmation still applies.",
                    "--replace",
                    true,
                ),
            ];
            common_file_fields(
                &mut fields,
                jobs(8),
                "Optional destination folder; blank writes resized copies beside sources.",
            );
            Tool {
                name: "justresize",
                title: "JustResize",
                summary: "Resize still images safely",
                fields,
            }
        }
        "justjson" => Tool {
            name: "justjson",
            title: "JustJSON",
            summary: "Format, validate, or query JSON",
            fields: vec![
                input(
                    "Input files / folders",
                    "Use semicolons between multiple inputs. Blank processes the current folder. Pipe JSON for stdin mode instead of opening this UI.",
                    true,
                    Some("."),
                    false,
                ),
                toggle(
                    "check",
                    "Validate only",
                    "Validate without writing. This action applies to this run only.",
                    "--check",
                    false,
                ),
                text(
                    "get",
                    "Get path",
                    "Print one value such as user.name or items[0].",
                    "",
                    "--get",
                    true,
                ),
                toggle(
                    "minify",
                    "Minify",
                    "Remove insignificant whitespace.",
                    "--minify",
                    true,
                ),
                toggle(
                    "sort",
                    "Sort keys",
                    "Sort object keys recursively.",
                    "--sort",
                    true,
                ),
                number(
                    "indent",
                    "Indent",
                    "Spaces per nesting level.",
                    2,
                    "--indent",
                    0,
                    8,
                    1,
                ),
                text(
                    "output",
                    "Output",
                    "Optional output directory; blank formats files in place.",
                    "",
                    "--output",
                    true,
                ),
                toggle(
                    "recursive",
                    "Recursive",
                    "Include nested folders.",
                    "--recursive",
                    true,
                ),
                toggle(
                    "dry_run",
                    "Dry run",
                    "Show files without writing. Applies to this run only.",
                    "--dry-run",
                    false,
                ),
            ],
        },
        "justpdf" => Tool {
            name: "justpdf",
            title: "JustPDF",
            summary: "Inspect and transform PDF files",
            fields: vec![
                choices(
                    "operation",
                    "Operation",
                    "Auto shows info for one PDF and merges multiple PDFs.",
                    "auto",
                    vec![
                        choice("Auto", "auto", &[]),
                        choice("Merge", "merge", &["merge"]),
                        choice("Split", "split", &["split"]),
                        choice("Extract", "extract", &["extract"]),
                        choice("Rotate", "rotate", &["rotate"]),
                        choice("Info", "info", &["info"]),
                    ],
                ),
                input(
                    "PDF files / folders",
                    "Required. Use semicolons between multiple inputs.",
                    true,
                    None,
                    true,
                ),
                text(
                    "output",
                    "Output",
                    "Output PDF path or split output directory.",
                    "",
                    "--output",
                    true,
                ),
                text(
                    "pages",
                    "Page range",
                    "One-based range such as 1-3,5,last. Required for Extract.",
                    "",
                    "--pages",
                    true,
                ),
                choices(
                    "degrees",
                    "Rotation",
                    "Clockwise rotation in degrees.",
                    "90",
                    vec![
                        choice("90 degrees", "90", &[]),
                        choice("180 degrees", "180", &["--degrees", "180"]),
                        choice("270 degrees", "270", &["--degrees", "270"]),
                    ],
                ),
                toggle(
                    "recursive",
                    "Recursive",
                    "Include nested folders.",
                    "--recursive",
                    true,
                ),
                toggle(
                    "dry_run",
                    "Dry run",
                    "Show planned outputs without writing. Applies to this run only.",
                    "--dry-run",
                    false,
                ),
            ],
        },
        "justport" => Tool {
            name: "justport",
            title: "JustPort",
            summary: "Inspect local port ownership",
            fields: vec![
                input(
                    "Ports",
                    "Required in the UI. Separate multiple port numbers with semicolons.",
                    true,
                    None,
                    true,
                ),
                toggle(
                    "all",
                    "Include UDP",
                    "Include UDP endpoints in addition to TCP.",
                    "--all",
                    true,
                ),
                toggle(
                    "json",
                    "JSON output",
                    "Emit machine-readable JSON for this run.",
                    "--json",
                    false,
                ),
                toggle(
                    "kill",
                    "Stop owners",
                    "Stop owning user processes after identity checks and confirmation. Never saved.",
                    "--kill",
                    false,
                ),
            ],
        },
        "justqr" => Tool {
            name: "justqr",
            title: "JustQR",
            summary: "Generate a ready-to-scan QR code",
            fields: vec![
                input(
                    "Text",
                    "Required QR payload. It is intentionally never saved. Semicolons remain literal because this is one value.",
                    false,
                    None,
                    true,
                ),
                text(
                    "output",
                    "Output",
                    "Output file path. Blank uses qr.png or qr.svg.",
                    "",
                    "--output",
                    true,
                ),
                choices(
                    "format",
                    "Format",
                    "PNG, scalable SVG, or a compact terminal rendering.",
                    "png",
                    vec![
                        choice("PNG file", "png", &[]),
                        choice("SVG file", "svg", &["--svg"]),
                        choice("Terminal", "terminal", &["--terminal"]),
                    ],
                ),
                number(
                    "width",
                    "PNG width",
                    "PNG width in pixels; ignored for SVG and terminal output.",
                    1024,
                    "--width",
                    64,
                    4096,
                    64,
                ),
                choices(
                    "error",
                    "Error correction",
                    "QR error correction level.",
                    "q",
                    vec![
                        choice("L — low", "l", &["--error", "l"]),
                        choice("M — medium", "m", &["--error", "m"]),
                        choice("Q — quartile", "q", &[]),
                        choice("H — high", "h", &["--error", "h"]),
                    ],
                ),
                number(
                    "margin",
                    "Quiet zone",
                    "Blank modules around the code.",
                    4,
                    "--margin",
                    0,
                    20,
                    1,
                ),
                text(
                    "dark",
                    "Foreground",
                    "Six- or eight-digit hex color.",
                    "#000000",
                    "--dark",
                    true,
                ),
                text(
                    "light",
                    "Background",
                    "Six- or eight-digit hex color.",
                    "#ffffff",
                    "--light",
                    true,
                ),
                toggle(
                    "dry_run",
                    "Dry run",
                    "Show resolved output without writing. Applies to this run only.",
                    "--dry-run",
                    false,
                ),
            ],
        },
        "justrmbg" => Tool {
            name: "justrmbg",
            title: "JustRMBG",
            summary: "Remove image backgrounds locally",
            fields: vec![
                input(
                    "Image / folder",
                    "Required unless Runtime check is on. Use semicolons for multiple images.",
                    true,
                    None,
                    false,
                ),
                text(
                    "output",
                    "Output file / folder",
                    "Output file for one input or output directory for a batch.",
                    "",
                    "--output",
                    true,
                ),
                choices(
                    "provider",
                    "Provider",
                    "Auto prefers acceleration and visibly falls back; explicit providers are strict.",
                    "auto",
                    vec![
                        choice("Auto", "auto", &[]),
                        choice("CPU", "cpu", &["--provider", "cpu"]),
                        choice("DirectML", "directml", &["--provider", "directml"]),
                        choice("CUDA", "cuda", &["--provider", "cuda"]),
                        choice("CoreML", "coreml", &["--provider", "coreml"]),
                    ],
                ),
                text(
                    "model",
                    "Model path",
                    "Optional local ONNX model path. Blank uses RMBG_MODEL or managed cache.",
                    "",
                    "--model",
                    true,
                ),
                toggle_default(
                    "download",
                    "Install dependencies",
                    "If missing, download the pinned model and managed runtime after Run.",
                    "--download",
                    false,
                    true,
                ),
                toggle(
                    "check",
                    "Runtime check",
                    "Test runtime/provider initialization without an image or model download. Applies to this run only.",
                    "--check",
                    false,
                ),
            ],
        },
        "justsvg" => Tool {
            name: "justsvg",
            title: "JustSVG",
            summary: "Optimize SVG files conservatively",
            fields: vec![
                input(
                    "Input files / folders",
                    "Use semicolons between multiple inputs. Blank processes the current folder. Pipe SVG for stdin mode instead of opening this UI.",
                    true,
                    Some("."),
                    false,
                ),
                number(
                    "precision",
                    "Precision",
                    "Decimal precision from 0 to 5.",
                    3,
                    "--precision",
                    0,
                    5,
                    1,
                ),
                toggle(
                    "single_pass",
                    "Single pass",
                    "Disable multipass optimization.",
                    "--single-pass",
                    true,
                ),
                text(
                    "output",
                    "Output",
                    "Optional output directory; blank safely replaces only when smaller.",
                    "",
                    "--output",
                    true,
                ),
                toggle(
                    "recursive",
                    "Recursive",
                    "Include nested folders.",
                    "--recursive",
                    true,
                ),
                toggle(
                    "dry_run",
                    "Dry run",
                    "Show planned outputs without writing. Applies to this run only.",
                    "--dry-run",
                    false,
                ),
            ],
        },
        "justzip" => Tool {
            name: "justzip",
            title: "JustZIP",
            summary: "Archive exactly what Git includes",
            fields: vec![
                input(
                    "Project folder",
                    "Blank archives the current folder.",
                    false,
                    Some("."),
                    false,
                ),
                text(
                    "output",
                    "Output",
                    "ZIP path or an existing destination directory.",
                    "",
                    "--output",
                    true,
                ),
                choices(
                    "compression",
                    "Compression",
                    "Choose speed versus archive size.",
                    "smallest",
                    vec![
                        choice("Smallest", "smallest", &[]),
                        choice("Balanced", "balanced", &["--compression", "balanced"]),
                        choice("Fast", "fast", &["--compression", "fast"]),
                    ],
                ),
                toggle(
                    "dry_run",
                    "Dry run",
                    "Show files without writing an archive. Applies to this run only.",
                    "--dry-run",
                    false,
                ),
            ],
        },
        "justcommit" => Tool {
            name: "justcommit",
            title: "JustCommit",
            summary: "Create an AI-written Git commit",
            fields: vec![
                input(
                    "Repository",
                    "Blank uses the current repository. Credentials are read from the environment and never stored here.",
                    false,
                    Some("."),
                    false,
                ),
                text(
                    "model",
                    "Model",
                    "OpenRouter model used for the bounded change digest.",
                    "google/gemini-2.5-flash-lite:nitro",
                    "--model",
                    true,
                ),
                choices(
                    "scope",
                    "Stage scope",
                    "Complete worktree stages everything; staged only preserves the existing index.",
                    "all",
                    vec![
                        choice("Complete worktree", "all", &[]),
                        choice("Staged only", "staged", &["--staged"]),
                    ],
                ),
                toggle(
                    "push",
                    "Push after commit",
                    "Push after a successful commit. This action is never saved.",
                    "--push",
                    false,
                ),
                toggle(
                    "dry_run",
                    "Dry run",
                    "Generate without committing or pushing. This action is never saved; complete-worktree mode still stages files.",
                    "--dry-run",
                    false,
                ),
                toggle(
                    "no_patches",
                    "Names only",
                    "Exclude bounded patch samples from the model request.",
                    "--no-patches",
                    true,
                ),
                number(
                    "timeout",
                    "Timeout",
                    "OpenRouter request timeout in seconds.",
                    45,
                    "--timeout",
                    1,
                    300,
                    5,
                ),
                toggle(
                    "repair",
                    "Repair on failure",
                    "Send a safe failure brief to the selected local agent. This action is never saved.",
                    "--repair",
                    false,
                ),
                choices(
                    "repair_agent",
                    "Repair agent",
                    "Agent used only when Repair on failure is enabled.",
                    "auto",
                    vec![
                        choice("Auto", "auto", &[]),
                        choice("Codex", "codex", &["--repair-with", "codex"]),
                        choice("Claude", "claude", &["--repair-with", "claude"]),
                    ],
                ),
            ],
        },
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app_for(name: &str) -> App {
        let temp = tempfile::tempdir().unwrap();
        let store = preferences::Store::load_from(temp.path().join("defaults.toml")).unwrap();
        App::new(spec(name).unwrap(), store)
    }

    #[test]
    fn every_non_dashboard_command_has_a_launcher() {
        for name in crate::commands::COMMANDS
            .iter()
            .map(|command| command.name)
            .filter(|name| !matches!(*name, "justbunt" | "justports" | "justready"))
        {
            assert!(supports(name), "missing launcher for {name}");
        }
    }

    #[test]
    fn default_jpg_command_is_explicitly_headless() {
        let app = app_for("justjpg");
        assert_eq!(app.command(), "justjpg .");
    }

    #[test]
    fn changed_defaults_are_in_the_command_and_persist() {
        let mut app = app_for("justjpg");
        app.selected = app
            .values
            .iter()
            .position(|value| value.field.id == "quality")
            .unwrap();
        app.values[app.selected].value = "92".into();
        app.save_selected();
        assert!(app.command().contains("--quality 92"));
        assert_eq!(app.store.get("justjpg", "quality"), Some("92"));
    }

    #[test]
    fn input_lists_support_quoted_semicolons() {
        assert_eq!(
            split_inputs("one.png;\"two;still.png\""),
            ["one.png", "two;still.png"]
        );
    }

    #[test]
    fn qr_payload_and_one_run_actions_are_not_persistent() {
        let app = app_for("justqr");
        assert!(
            !app.values
                .iter()
                .find(|value| value.field.id == "input")
                .unwrap()
                .field
                .persistent
        );
        assert!(
            !app.values
                .iter()
                .find(|value| value.field.id == "dry_run")
                .unwrap()
                .field
                .persistent
        );
    }

    #[test]
    fn launcher_frame_shows_saved_scope_and_headless_command() {
        use ratatui::{Terminal, backend::TestBackend};

        let mut app = app_for("justjpg");
        let mut terminal = Terminal::new(TestBackend::new(100, 28)).unwrap();
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();
        let rendered = terminal.backend().to_string();
        assert!(rendered.contains("JustJPG"));
        assert!(rendered.contains("saved"));
        assert!(rendered.contains("Headless: justjpg ."));
    }

    #[test]
    fn every_launcher_renders_at_a_standard_eighty_column_terminal() {
        use ratatui::{Terminal, backend::TestBackend};

        for name in crate::commands::COMMANDS
            .iter()
            .map(|command| command.name)
            .filter(|name| supports(name))
        {
            let mut app = app_for(name);
            let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
            terminal.draw(|frame| draw(frame, &mut app)).unwrap();
            let rendered = terminal.backend().to_string();
            assert!(rendered.contains("Headless:"), "missing footer for {name}");
            assert!(
                rendered.contains(app.tool.title),
                "missing title for {name}"
            );
        }
    }

    #[test]
    fn incompatible_saved_choices_generate_a_valid_precedence() {
        let mut app = app_for("justresize");
        app.values
            .iter_mut()
            .find(|value| value.field.id == "max")
            .unwrap()
            .value = "1600".into();
        app.values
            .iter_mut()
            .find(|value| value.field.id == "width")
            .unwrap()
            .value = "800".into();
        app.values
            .iter_mut()
            .find(|value| value.field.id == "output")
            .unwrap()
            .value = "out".into();
        app.values
            .iter_mut()
            .find(|value| value.field.id == "replace")
            .unwrap()
            .value = "true".into();
        let command = app.command();
        assert!(command.contains("--width 800"));
        assert!(command.contains("--output out"));
        assert!(!command.contains("--max"));
        assert!(!command.contains("--replace"));
    }

    #[test]
    fn image_launchers_state_destination_and_overwrite_policy() {
        let webp = app_for("justwebp");
        let (output, overwrite) = webp.output_policy();
        assert!(output.contains("beside each source as <name>.webp"));
        assert!(overwrite.contains("original is removed"));

        let optimize = app_for("justoptimize");
        let (output, overwrite) = optimize.output_policy();
        assert!(output.contains("<name>-optimized.<best>"));
        assert!(overwrite.contains("source kept"));
    }

    #[test]
    fn rmbg_launcher_approves_pinned_dependency_download_visibly() {
        let mut app = app_for("justrmbg");
        app.values
            .iter_mut()
            .find(|value| value.field.id == "input")
            .unwrap()
            .value = "photo.png".into();
        assert!(app.command().contains("--download"));
        assert!(
            app.values
                .iter()
                .find(|value| value.field.id == "download")
                .is_some_and(|value| !value.field.persistent && value.value == "true")
        );
    }
}
