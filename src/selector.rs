use std::{ffi::OsString, io, time::Duration};

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{
    DefaultTerminal, Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, TableState, Wrap},
};

use crate::commands::{self, COMMANDS};
use crate::error::{ToolError, ToolResult};

fn list() {
    let width = COMMANDS
        .iter()
        .map(|command| command.name.len())
        .max()
        .unwrap_or(4);
    println!("just — run one of the compiled just* tools\n");
    for command in COMMANDS {
        println!(
            "  {:width$}  {}",
            command.name,
            command.description,
            width = width
        );
    }
    if let Ok(directory) = crate::pathing::current_bin_directory()
        && !crate::pathing::contains(&directory)
    {
        println!("\n  Add To Path  add {} to your PATH", directory.display());
    }
    println!("\nrun: just <tool> [args]   (e.g. `just qr hello`, `just help video`)");
    println!("defaults: just --defaults-path   (interactive changes save automatically)");
}

fn normalized_tool(value: &str) -> String {
    if value.starts_with("just") {
        value.to_ascii_lowercase()
    } else {
        format!("just{}", value.to_ascii_lowercase())
    }
}

pub fn run(args: Vec<OsString>) -> ToolResult {
    if args.len() == 1 && (args[0] == "-V" || args[0] == "--version") {
        println!("just {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if args.first().is_some_and(|arg| arg == "install") {
        return crate::install::run(args.into_iter().skip(1).collect());
    }
    if args
        .first()
        .is_some_and(|arg| arg == "add-to-path" || arg == "--add-to-path")
    {
        if args.len() > 1 {
            return Err(ToolError::usage(
                "just",
                "add-to-path does not take arguments",
            ));
        }
        return crate::pathing::add(&crate::pathing::current_bin_directory()?);
    }
    if args.first().is_some_and(|arg| arg == "--defaults-path") {
        if args.len() > 1 {
            return Err(ToolError::usage(
                "just",
                "--defaults-path does not take arguments",
            ));
        }
        println!(
            "{}",
            crate::preferences::path()
                .map_err(|error| ToolError::new("just", format!("{error:#}")))?
                .display()
        );
        return Ok(());
    }
    if args
        .first()
        .is_some_and(|arg| arg == "-h" || arg == "--help" || arg == "list")
    {
        if args.len() > 1 {
            return Err(ToolError::usage(
                "just",
                "--help/list does not take extra arguments",
            ));
        }
        list();
        return Ok(());
    }
    if args.first().is_some_and(|arg| arg == "help") {
        if args.len() == 1 {
            list();
            return Ok(());
        }
        if args.len() > 2 {
            return Err(ToolError::usage(
                "just",
                "help accepts at most one tool name",
            ));
        }
        let requested = args[1].to_string_lossy();
        return commands::dispatch(&normalized_tool(&requested), vec![OsString::from("--help")]);
    }
    if let Some(first) = args.first() {
        let text = first
            .to_str()
            .ok_or_else(|| ToolError::usage("just", "tool name must be valid UTF-8"))?;
        if text.starts_with('-') {
            return Err(ToolError::usage("just", format!("unknown option: {text}")));
        }
        return commands::dispatch(&normalized_tool(text), args.into_iter().skip(1).collect());
    }
    if !crate::common::stdin_is_terminal() || !crate::common::stdout_is_terminal() {
        list();
        return Ok(());
    }
    let path_action = crate::pathing::current_bin_directory()
        .ok()
        .filter(|directory| !crate::pathing::contains(directory));
    let Some(selection) = choose(path_action.as_deref())
        .map_err(|error| ToolError::new("just", format!("terminal UI failed: {error}")))?
    else {
        return Ok(());
    };
    if selection == COMMANDS.len() {
        crate::pathing::add(path_action.as_deref().expect("PATH action is present"))
    } else {
        commands::dispatch(COMMANDS[selection].name, Vec::new())
    }
}

struct App {
    selected: usize,
    table: TableState,
    path_action: Option<String>,
    quit: bool,
    confirmed: bool,
    help: bool,
}

fn choose(path_action: Option<&std::path::Path>) -> io::Result<Option<usize>> {
    let mut app = App {
        selected: 0,
        table: TableState::default(),
        path_action: path_action.map(|path| path.display().to_string()),
        quit: false,
        confirmed: false,
        help: false,
    };
    ratatui::run(|terminal| run_selector(terminal, &mut app))?;
    Ok(app.confirmed.then_some(app.selected))
}

fn run_selector(terminal: &mut DefaultTerminal, app: &mut App) -> io::Result<()> {
    while !app.quit {
        terminal.draw(|frame| draw(frame, app))?;
        if event::poll(Duration::from_millis(250))?
            && let Event::Key(key) = event::read()?
            && matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat)
        {
            if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                app.quit = true;
                continue;
            }
            if app.help {
                if matches!(
                    key.code,
                    KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q')
                ) {
                    app.help = false;
                }
                continue;
            }
            let count = COMMANDS.len() + usize::from(app.path_action.is_some());
            match key.code {
                KeyCode::Up | KeyCode::Char('k') => app.selected = app.selected.saturating_sub(1),
                KeyCode::Down | KeyCode::Char('j') => {
                    app.selected = (app.selected + 1).min(count.saturating_sub(1))
                }
                KeyCode::Home => app.selected = 0,
                KeyCode::End => app.selected = count.saturating_sub(1),
                KeyCode::Char('?') => app.help = true,
                KeyCode::Enter => {
                    app.confirmed = true;
                    app.quit = true;
                }
                KeyCode::Esc | KeyCode::Char('q') => app.quit = true,
                _ => {}
            }
        }
    }
    Ok(())
}

fn draw(frame: &mut Frame<'_>, app: &mut App) {
    let [header, table_area, details, footer] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(8),
        Constraint::Length(5),
        Constraint::Length(3),
    ])
    .areas(frame.area());
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!(" {} tools ", COMMANDS.len()),
                crate::console_ui::bold(crate::console_ui::GOOD),
            ),
            Span::styled(
                "  one binary · native aliases · saved defaults ",
                crate::console_ui::MUTED,
            ),
        ]))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(Span::styled(" JustTools ", Style::default().bold())),
        ),
        header,
    );
    let mut rows = COMMANDS
        .iter()
        .map(|command| {
            Row::new(vec![
                Cell::from(command.name),
                Cell::from(command.description),
            ])
        })
        .collect::<Vec<_>>();
    if let Some(path) = &app.path_action {
        rows.push(Row::new(vec![
            Cell::from("Add To Path"),
            Cell::from(format!("add {path} to PATH")),
        ]));
    }
    let table = Table::new(rows, [Constraint::Length(15), Constraint::Min(30)])
        .header(
            Row::new(["Tool", "Purpose"])
                .style(Style::default().fg(Color::White).bold())
                .bottom_margin(1),
        )
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(crate::console_ui::ACCENT)
                .title(" Choose a tool "),
        )
        .row_highlight_style(crate::console_ui::selected())
        .highlight_symbol("› ");
    app.table.select(Some(app.selected));
    frame.render_stateful_widget(table, table_area, &mut app.table);
    let detail = if app.selected < COMMANDS.len() {
        format!(
            "{}\n\nEnter opens its interactive console. Any explicit arguments keep using the direct headless command.",
            COMMANDS[app.selected].description
        )
    } else {
        "Make every just* alias available from new terminal sessions.".into()
    };
    frame.render_widget(
        Paragraph::new(detail)
            .wrap(Wrap { trim: true })
            .block(Block::default().borders(Borders::ALL).title(" Details ")),
        details,
    );
    let command = if app.selected < COMMANDS.len() {
        format!(
            "just {}",
            COMMANDS[app.selected].name.trim_start_matches("just")
        )
    } else {
        "just add-to-path".into()
    };
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                "↑↓/jk move  Enter open  ? help  q quit",
                crate::console_ui::MUTED,
            )),
            Line::from(vec![
                Span::styled("Command: ", crate::console_ui::MUTED),
                Span::styled(
                    command,
                    crate::console_ui::bold(crate::console_ui::SECONDARY),
                ),
            ]),
        ]),
        footer,
    );
    if app.help {
        let area = centered(frame.area(), 72, 11);
        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new(vec![
                Line::styled(
                    "JustTools console behavior",
                    crate::console_ui::bold(crate::console_ui::ACCENT),
                ),
                Line::raw(""),
                Line::raw("  Bare just* commands open a consistent interactive launcher."),
                Line::raw("  Explicit arguments and piped input stay fully headless."),
                Line::raw("  Changed settings marked saved become that tool's next default."),
                Line::raw("  Every launcher shows its equivalent headless command at the bottom."),
                Line::raw(""),
                Line::styled("Press ? / q / Esc to close help", crate::console_ui::MUTED),
            ])
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(crate::console_ui::ACCENT)
                    .title(" JustTools help "),
            )
            .wrap(Wrap { trim: true }),
            area,
        );
    }
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
