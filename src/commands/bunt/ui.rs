use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Cell, Clear, Gauge, Paragraph, Row, Table, Wrap},
};

use super::{
    app::{App, Mode},
    model::Runtime,
};
use crate::console_ui::{
    ACCENT as ACCENT_COLOR, GOOD as TARGET_COLOR, MUTED as MUTED_COLOR, SECONDARY as SAFETY_COLOR,
    SELECTED_BG, WARNING as EXCLUDED_COLOR,
};

pub fn draw(frame: &mut Frame<'_>, app: &mut App) {
    let area = frame.area();
    let detail_height = if area.height >= 28 { 8 } else { 6 };
    let [header_area, table_area, details_area, footer_area] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(6),
        Constraint::Length(detail_height),
        Constraint::Length(3),
    ])
    .areas(area);

    render_header(frame, app, header_area);
    render_table(frame, app, table_area);
    render_details(frame, app, details_area);
    render_footer(frame, app, footer_area);

    match app.mode {
        Mode::Confirm => render_confirmation(frame, app),
        Mode::Closing => render_closing(frame, app),
        Mode::Help => render_help(frame, app),
        _ => {}
    }
}

fn render_header(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let (targets, excluded, safety) = app.counts();
    let mut spans = vec![
        Span::styled(format!(" {targets} targets "), bold(TARGET_COLOR)),
        Span::styled(format!(" {excluded} excluded "), EXCLUDED_COLOR),
    ];
    if safety > 0 {
        spans.push(Span::styled(format!(" {safety} safety "), SAFETY_COLOR));
    }
    spans.extend([
        Span::raw("  view:"),
        Span::styled(app.view_filter.label(), bold(ACCENT_COLOR)),
        Span::raw("  runtime:"),
        Span::styled(app.runtime_filter.label(), bold(ACCENT_COLOR)),
        Span::raw("  sort:"),
        Span::styled(app.sort_key.label(), bold(ACCENT_COLOR)),
    ]);
    if app.mode == Mode::Search || !app.query.is_empty() {
        spans.extend([
            Span::raw("  /"),
            Span::styled(app.query.as_str(), bold(Color::White)),
            Span::styled(
                if app.mode == Mode::Search { "▌" } else { "" },
                ACCENT_COLOR,
            ),
        ]);
    }

    frame.render_widget(
        Paragraph::new(Line::from(spans)).block(
            Block::default()
                .borders(Borders::ALL)
                .title(Span::styled(" bunt ", Style::default().bold())),
        ),
        area,
    );
}

fn render_table(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let visible = app.visible_indices();
    let rows = visible
        .iter()
        .filter_map(|index| app.processes.get(*index))
        .map(|process| {
            let protection = app.protection(process);
            let (marker, marker_style) = match protection.as_ref() {
                None => ("●", Style::default().fg(TARGET_COLOR)),
                Some(protection) if protection.is_excluded() => {
                    ("E", Style::default().fg(EXCLUDED_COLOR).bold())
                }
                Some(_) => ("S", Style::default().fg(SAFETY_COLOR).bold()),
            };
            let runtime_style = runtime_style(process.runtime);
            let row_style = if protection.is_some() {
                Style::default().fg(MUTED_COLOR)
            } else {
                Style::default()
            };
            let workload = if process.project_name.is_empty() {
                process.workload_label.clone()
            } else {
                format!("{}  ·  {}", process.project_name, process.workload_label)
            };
            Row::new(vec![
                Cell::from(marker).style(marker_style),
                Cell::from(process.runtime.to_string()).style(runtime_style),
                Cell::from(process.pid.to_string()),
                Cell::from(format!("{:>5.1}%", process.cpu_percent)),
                Cell::from(format_bytes(process.memory_bytes)),
                Cell::from(format_age(process.run_time)),
                Cell::from(workload),
            ])
            .style(row_style)
        })
        .collect::<Vec<_>>();

    let header = Row::new(["", "Runtime", "PID", "CPU", "Memory", "Age", "Workload"])
        .style(Style::default().fg(Color::White).bold())
        .bottom_margin(1);
    let widths = [
        Constraint::Length(2),
        Constraint::Length(8),
        Constraint::Length(7),
        Constraint::Length(7),
        Constraint::Length(10),
        Constraint::Length(9),
        Constraint::Min(18),
    ];
    let title = format!(
        " Processes  {} visible / {} detected ",
        visible.len(),
        app.processes.len()
    );
    let table = Table::new(rows, widths)
        .header(header)
        .block(Block::default().borders(Borders::ALL).title(title))
        .row_highlight_style(
            Style::default()
                .bg(SELECTED_BG)
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("› ");
    app.table_state
        .select((!visible.is_empty()).then_some(app.selected.min(visible.len().saturating_sub(1))));
    frame.render_stateful_widget(table, area, &mut app.table_state);
}

fn render_details(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let block = Block::default().borders(Borders::ALL).title(" Details ");
    let Some(process) = app.selected_process() else {
        frame.render_widget(
            Paragraph::new("No process matches the current filters")
                .alignment(Alignment::Center)
                .block(block),
            area,
        );
        return;
    };

    let protection = app.protection(process);
    let protection_line = match protection.as_ref() {
        None => Line::from(vec![
            Span::styled("TARGET", bold(TARGET_COLOR)),
            Span::raw(" — included in Kill All"),
        ]),
        Some(protection) if protection.is_excluded() => Line::from(vec![
            Span::styled("EXCLUDED", bold(EXCLUDED_COLOR)),
            Span::raw(format!(" — {}", protection.label())),
        ]),
        Some(protection) => Line::from(vec![
            Span::styled("SAFETY", bold(SAFETY_COLOR)),
            Span::raw(format!(" — {}", protection.label())),
        ]),
    };
    let lines = vec![
        Line::from(vec![
            Span::styled(
                process.runtime.to_string(),
                runtime_style(process.runtime).bold(),
            ),
            Span::raw(format!(
                "  PID {}  PPID {}  {}  CPU {:.1}%  RSS {}  virtual {}",
                process.pid,
                process
                    .parent_pid
                    .map(|pid| pid.to_string())
                    .unwrap_or_else(|| "—".into()),
                process.status,
                process.cpu_percent,
                format_bytes(process.memory_bytes),
                format_bytes(process.virtual_memory_bytes),
            )),
        ]),
        Line::from(format!(
            "Project: {}  |  Root: {}",
            process.project_name,
            process.project_root.as_deref().unwrap_or("unavailable")
        )),
        Line::from(format!(
            "CWD: {}  |  EXE: {}",
            process.cwd.as_deref().unwrap_or("unavailable"),
            process.executable.as_deref().unwrap_or("unavailable")
        )),
        Line::from(format!(
            "Command: {}",
            if process.command.is_empty() {
                "unavailable"
            } else {
                &process.command
            }
        )),
        Line::from(format!(
            "Age: {}  |  I/O since refresh: read {} / wrote {}  |  {} args",
            format_age(process.run_time),
            format_bytes(process.disk_read_bytes),
            format_bytes(process.disk_written_bytes),
            process.args.len(),
        )),
        protection_line,
    ];
    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .block(block)
            .wrap(Wrap { trim: true }),
        area,
    );
}

fn render_footer(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let keys = match app.mode {
        Mode::Search => "type to filter  Enter keep  Esc clear",
        Mode::Normal => {
            "↑↓/jk move  / filter  Tab view  1-4 runtime  e exclude  x kill  K kill all  ? help  q quit"
        }
        Mode::Confirm => "y/Enter confirm  n/Esc cancel",
        Mode::Closing => "closing safely… the process list and progress remain live",
        Mode::Help => "?/q/Esc close help",
    };
    let status = app.status_text();
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(keys, MUTED_COLOR)),
            Line::from(Span::styled(status, ACCENT_COLOR)),
            Line::from(vec![
                Span::styled("Headless: ", MUTED_COLOR),
                Span::styled("justbunt --snapshot", bold(Color::Rgb(194, 145, 255))),
            ]),
        ]),
        area,
    );
}

fn render_closing(frame: &mut Frame<'_>, app: &App) {
    let Some(operation) = app.close_operation.as_ref() else {
        return;
    };
    let area = centered(frame.area(), 76, 12);
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(ACCENT_COLOR)
        .title(" Closing processes ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let [summary_area, gauge_area, note_area] = Layout::vertical([
        Constraint::Length(5),
        Constraint::Length(2),
        Constraint::Min(1),
    ])
    .areas(inner);

    let spinner = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
    let frame_index = (operation.started_at.elapsed().as_millis() / 80) as usize % spinner.len();
    let remaining = operation.remaining();
    let timing = if operation.stage == super::app::ClosingStage::Graceful {
        format!("  {:.1}s remaining", remaining.as_secs_f64())
    } else {
        String::new()
    };
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled(format!(" {} ", spinner[frame_index]), bold(ACCENT_COLOR)),
                Span::styled(operation.stage_label(), Style::default().bold()),
                Span::styled(timing, MUTED_COLOR),
            ]),
            Line::from(""),
            Line::from(format!(
                " Requested {}  ·  Revalidated {}  ·  Graceful {}  ·  Force {}",
                operation.requested,
                operation.eligible_count,
                operation.graceful_count,
                operation.force_requested,
            )),
            Line::from(format!(
                " Changed/protected {}  ·  Signal errors {}",
                operation.skipped, operation.failed,
            )),
        ]),
        summary_area,
    );

    let progress = operation.progress();
    frame.render_widget(
        Gauge::default()
            .ratio(progress)
            .label(format!("{:>3}%", (progress * 100.0).round() as u8))
            .gauge_style(Style::default().fg(ACCENT_COLOR).bg(Color::Rgb(28, 35, 48))),
        gauge_area,
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("The TUI is still responsive. ", TARGET_COLOR),
            Span::styled(
                "Only the confirmed snapshot is being closed; new processes are never added.",
                MUTED_COLOR,
            ),
        ]))
        .wrap(Wrap { trim: true }),
        note_area,
    );
}

fn render_confirmation(frame: &mut Frame<'_>, app: &App) {
    let Some(pending) = app.pending_kill.as_ref() else {
        return;
    };
    let shown = pending.targets.len().min(7);
    let height = u16::try_from(shown).unwrap_or(7) + 8;
    let area = centered(frame.area(), 82, height);
    frame.render_widget(Clear, area);

    let heading = if pending.all {
        format!(
            "Kill all {} non-protected processes?",
            pending.targets.len()
        )
    } else {
        "Kill this process?".into()
    };
    let mut lines = vec![
        Line::from(Span::styled(heading, bold(Color::Red))),
        Line::from(""),
    ];
    lines.extend(
        pending
            .targets
            .iter()
            .take(shown)
            .map(|preview| Line::from(format!("  {}", preview.display))),
    );
    if pending.targets.len() > shown {
        lines.push(Line::from(format!(
            "  … and {} more",
            pending.targets.len() - shown
        )));
    }
    lines.extend([
        Line::from(""),
        Line::from("PID, start time, workload, and exclusions are rechecked first."),
        Line::from("Unix: TERM then automatic force. Windows: native termination."),
        Line::from(""),
        Line::from(vec![
            Span::styled(" y / Enter ", bold(TARGET_COLOR)),
            Span::raw("confirm     "),
            Span::styled(" n / Esc ", bold(EXCLUDED_COLOR)),
            Span::raw("cancel"),
        ]),
    ]);

    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Color::Red)
                    .title(" Confirm "),
            )
            .wrap(Wrap { trim: true }),
        area,
    );
}

fn render_help(frame: &mut Frame<'_>, app: &App) {
    let area = centered(frame.area(), 88, 25);
    frame.render_widget(Clear, area);
    let lines = vec![
        Line::from(Span::styled("Fast path", bold(ACCENT_COLOR))),
        Line::from("  e       Toggle a persistent workload exclusion immediately"),
        Line::from("  K       Kill the current snapshot of every non-protected runtime"),
        Line::from("  x       Kill only the selected process"),
        Line::from(""),
        Line::from(Span::styled("Navigation and views", bold(ACCENT_COLOR))),
        Line::from("  ↑↓/jk   Move                 g/G  First/last"),
        Line::from("  Tab     All → targets → protected"),
        Line::from("  1/2/3/4 All / Node / Bun / Python"),
        Line::from("  s       Name → CPU → memory → age sort"),
        Line::from("  r       Refresh now          q  Quit"),
        Line::from(""),
        Line::from(Span::styled("Smart search", bold(ACCENT_COLOR))),
        Line::from("  / vite                  fuzzy text search"),
        Line::from("  / python project:api    combine filters"),
        Line::from("  / cmd:uvicorn -test     field filters and negation"),
        Line::from("  Fields: runtime:, pid:, project:, cwd:, cmd:, status:, is:"),
        Line::from("  States: is:target, is:protected, is:excluded, is:ancestor"),
        Line::from(""),
        Line::from(Span::styled("Safety", bold(ACCENT_COLOR))),
        Line::from("  S is automatic safety protection for bunt's launcher ancestry."),
        Line::from("  E is a persistent rule stored in the human-editable TOML file."),
        Line::from(format!("  Config: {}", app.store.path().display())),
        Line::from(""),
        Line::from(Span::styled("Press ? / q / Esc to close", MUTED_COLOR)),
    ];
    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(ACCENT_COLOR)
                    .title(" Help "),
            )
            .wrap(Wrap { trim: true }),
        area,
    );
}

fn centered(outer: Rect, desired_width: u16, desired_height: u16) -> Rect {
    let width = desired_width.min(outer.width.saturating_sub(2)).max(1);
    let height = desired_height.min(outer.height.saturating_sub(2)).max(1);
    Rect::new(
        outer.x + outer.width.saturating_sub(width) / 2,
        outer.y + outer.height.saturating_sub(height) / 2,
        width,
        height,
    )
}

fn runtime_style(runtime: Runtime) -> Style {
    match runtime {
        Runtime::Node => Style::default().fg(Color::Rgb(104, 190, 89)),
        Runtime::Bun => Style::default().fg(Color::Rgb(238, 175, 232)),
        Runtime::Python => Style::default().fg(Color::Rgb(255, 211, 92)),
    }
}

fn bold(color: Color) -> Style {
    Style::default().fg(color).add_modifier(Modifier::BOLD)
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{} {}", bytes, UNITS[unit])
    } else if value >= 10.0 {
        format!("{value:.0} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn format_age(seconds: u64) -> String {
    if seconds < 60 {
        return format!("{seconds}s");
    }
    if seconds < 3_600 {
        return format!("{}m {}s", seconds / 60, seconds % 60);
    }
    if seconds < 86_400 {
        return format!("{}h {}m", seconds / 3_600, (seconds % 3_600) / 60);
    }
    format!("{}d {}h", seconds / 86_400, (seconds % 86_400) / 3_600)
}

#[cfg(test)]
mod tests {
    use super::super::{
        app::{CloseOperation, PendingKill},
        config::ConfigStore,
        process::ProcessScanner,
    };
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    #[test]
    fn byte_format_is_compact() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1_048_576), "1.0 MB");
    }

    #[test]
    fn age_format_is_compact() {
        assert_eq!(format_age(59), "59s");
        assert_eq!(format_age(3_661), "1h 1m");
    }

    #[test]
    fn complete_ui_help_and_closing_progress_render_in_a_test_terminal() {
        let temp = tempfile::tempdir().unwrap();
        let store = ConfigStore::load_from(temp.path().join("config.toml")).unwrap();
        let mut app = App::new(ProcessScanner::new(), store);
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|frame| draw(frame, &mut app)).unwrap();
        assert!(
            terminal
                .backend()
                .to_string()
                .contains("Headless: justbunt --snapshot")
        );
        app.mode = Mode::Help;
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();
        app.close_operation = Some(CloseOperation::new(PendingKill {
            targets: Vec::new(),
            all: true,
        }));
        app.mode = Mode::Closing;
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    }
}
