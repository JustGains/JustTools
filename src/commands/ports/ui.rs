use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, Wrap},
};

use super::app::{App, Focus, Mode};
use crate::console_ui::{
    ACCENT as ACCENT_COLOR, GOOD as DEV_COLOR, MUTED as MUTED_COLOR, SECONDARY as FRAMEWORK_COLOR,
    SELECTED_BG,
};

pub fn draw(frame: &mut Frame<'_>, app: &mut App) {
    let detail_height = if frame.area().height >= 31 { 8 } else { 6 };
    let recent_height = if frame.area().height >= 31 { 7 } else { 5 };
    let [
        header_area,
        table_area,
        recent_area,
        details_area,
        footer_area,
    ] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(6),
        Constraint::Length(recent_height),
        Constraint::Length(detail_height),
        Constraint::Length(3),
    ])
    .areas(frame.area());

    render_header(frame, app, header_area);
    render_table(frame, app, table_area);
    render_recent(frame, app, recent_area);
    render_details(frame, app, details_area);
    render_footer(frame, app, footer_area);
    match app.mode {
        Mode::ConfirmKill => render_kill_confirmation(frame, app),
        Mode::Help => render_help(frame),
        _ => {}
    }
}

fn render_header(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let (development, all) = app.counts();
    let mut spans = vec![
        Span::styled(format!(" {development} dev servers "), bold(DEV_COLOR)),
        Span::styled(format!(" {all} TCP listeners "), MUTED_COLOR),
        Span::styled(format!(" {} launch again ", app.recent.len()), MUTED_COLOR),
        Span::raw("  view:"),
        Span::styled(app.view.label(), bold(ACCENT_COLOR)),
    ];
    if area.width >= 100 {
        spans.extend([Span::raw("  refresh:"), Span::styled("2s", ACCENT_COLOR)]);
    }
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
                .title(Span::styled(" JustPorts ", Style::default().bold())),
        ),
        area,
    );
}

fn render_table(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let visible = app.visible_indices();
    let rows = visible
        .iter()
        .filter_map(|index| app.servers.get(*index))
        .map(|server| {
            let marker = if server.is_dev_server { "●" } else { "○" };
            let marker_style = if server.is_dev_server {
                Style::default().fg(DEV_COLOR)
            } else {
                Style::default().fg(MUTED_COLOR)
            };
            Row::new(vec![
                Cell::from(marker).style(marker_style),
                Cell::from(server.project_name.clone()),
                Cell::from(server.url.clone()).style(Style::default().fg(ACCENT_COLOR)),
                Cell::from(server.framework.clone()).style(Style::default().fg(FRAMEWORK_COLOR)),
                Cell::from(server.pid.to_string()),
                Cell::from(server.process_name.clone()),
            ])
        })
        .collect::<Vec<_>>();
    let header = Row::new(["", "Project", "URL", "Stack", "PID", "Process"])
        .style(Style::default().fg(Color::White).bold())
        .bottom_margin(1);
    let widths = [
        Constraint::Length(2),
        Constraint::Percentage(22),
        Constraint::Percentage(30),
        Constraint::Percentage(16),
        Constraint::Length(7),
        Constraint::Min(12),
    ];
    let table = Table::new(rows, widths)
        .header(header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(if app.focus == Focus::Active {
                    ACCENT_COLOR
                } else {
                    MUTED_COLOR
                })
                .title(format!(" Running Now  {} visible ", visible.len())),
        )
        .row_highlight_style(highlight_style(app.focus == Focus::Active))
        .highlight_symbol("› ");
    app.table_state
        .select((!visible.is_empty()).then_some(app.selected.min(visible.len() - 1)));
    frame.render_stateful_widget(table, area, &mut app.table_state);
}

fn render_recent(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let visible = app.recent_visible_indices();
    let rows = visible
        .iter()
        .filter_map(|index| app.recent.get(*index))
        .map(|server| {
            Row::new(vec![
                Cell::from("◷").style(Style::default().fg(MUTED_COLOR)),
                Cell::from(server.project_name.clone()),
                Cell::from(server.port.to_string()).style(Style::default().fg(ACCENT_COLOR)),
                Cell::from(server.framework.clone()).style(Style::default().fg(FRAMEWORK_COLOR)),
                Cell::from(server.launch_label().to_owned()),
                Cell::from(format_last_seen(server.last_seen)),
            ])
        })
        .collect::<Vec<_>>();
    let header = Row::new([
        "",
        "Saved project",
        "Port",
        "Stack",
        "Start command",
        "Last",
    ])
    .style(Style::default().fg(Color::White).bold());
    let widths = [
        Constraint::Length(2),
        Constraint::Percentage(24),
        Constraint::Length(7),
        Constraint::Percentage(16),
        Constraint::Percentage(32),
        Constraint::Min(10),
    ];
    let table = Table::new(rows, widths)
        .header(header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(if app.focus == Focus::Recent {
                    ACCENT_COLOR
                } else {
                    MUTED_COLOR
                })
                .title(" Launch Again  Enter to start "),
        )
        .row_highlight_style(highlight_style(app.focus == Focus::Recent))
        .highlight_symbol("› ");
    app.recent_table_state.select(
        (!visible.is_empty()).then_some(app.recent_selected.min(visible.len().saturating_sub(1))),
    );
    frame.render_stateful_widget(table, area, &mut app.recent_table_state);
}

fn render_details(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let block = Block::default().borders(Borders::ALL).title(" Details ");
    if app.focus == Focus::Recent {
        let Some(server) = app.selected_recent() else {
            frame.render_widget(
                Paragraph::new(
                    "Servers are saved automatically and appear here when they stop running",
                )
                .alignment(Alignment::Center)
                .block(block),
                area,
            );
            return;
        };
        let lines = vec![
            Line::from(vec![
                Span::styled("SAVED", bold(MUTED_COLOR)),
                Span::raw(format!(
                    "  {}  ·  previous port {}  ·  {}  ·  seen {}",
                    server.project_name,
                    server.port,
                    server.framework,
                    format_last_seen(server.last_seen),
                )),
            ]),
            Line::from(format!("Previous URL: {}", server.url)),
            Line::from(format!(
                "Project: {}",
                server.project_root.as_deref().unwrap_or("unavailable")
            )),
            Line::from(format!("Start: {}", server.launch_label())),
            Line::from("Enter / s launches this project again from its saved working directory"),
        ];
        frame.render_widget(
            Paragraph::new(Text::from(lines))
                .block(block)
                .wrap(Wrap { trim: true }),
            area,
        );
        return;
    }
    let Some(server) = app.selected_server() else {
        frame.render_widget(
            Paragraph::new(if app.servers.is_empty() {
                "No TCP listeners detected — this view refreshes automatically"
            } else {
                "No development servers match this view — press Tab to show all listeners"
            })
            .alignment(Alignment::Center)
            .block(block),
            area,
        );
        return;
    };
    let state = if server.is_dev_server {
        Span::styled("DEV SERVER", bold(DEV_COLOR))
    } else {
        Span::styled("LISTENER", bold(MUTED_COLOR))
    };
    let lines = vec![
        Line::from(vec![
            state,
            Span::raw(format!(
                "  Port {}  ·  PID {}  ·  {}  ·  age {}  ·  RSS {}",
                server.port,
                server.pid,
                server.framework,
                format_age(server.run_time_seconds),
                format_bytes(server.memory_bytes),
            )),
        ]),
        Line::from(format!(
            "URL: {}  |  Bound: {}",
            server.url,
            server.addresses.join(", ")
        )),
        Line::from(format!(
            "Project: {}  |  Root: {}",
            server.project_name,
            server.project_root.as_deref().unwrap_or("unavailable")
        )),
        Line::from(format!(
            "CWD: {}  |  EXE: {}",
            server.cwd.as_deref().unwrap_or("unavailable"),
            server.executable.as_deref().unwrap_or("unavailable")
        )),
        Line::from(format!(
            "Command: {}",
            if server.command.is_empty() {
                "unavailable"
            } else {
                &server.command
            }
        )),
        Line::from(format!("Detected: {}", server.detection_reason)),
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
        Mode::ConfirmKill => "y / Enter stop selected service  n / Esc cancel",
        _ => "↑↓ move  Tab area  Enter open/start  K stop  / filter  a all  ? help  q quit",
    };
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(keys, MUTED_COLOR)),
            Line::from(Span::styled(app.status_text(), ACCENT_COLOR)),
            Line::from(vec![
                Span::styled("Headless: ", MUTED_COLOR),
                Span::styled(
                    if matches!(app.view, super::app::View::All) {
                        "justports --snapshot --all"
                    } else {
                        "justports --snapshot"
                    },
                    bold(FRAMEWORK_COLOR),
                ),
            ]),
        ]),
        area,
    );
}

fn render_help(frame: &mut Frame<'_>) {
    let area = centered(frame.area(), 80, 19);
    frame.render_widget(Clear, area);
    let lines = vec![
        Line::from(Span::styled("Open and explore", bold(ACCENT_COLOR))),
        Line::from("  Enter       Open Running Now or start the selected Launch Again project"),
        Line::from("  o           Open the selected active or remembered URL"),
        Line::from("  p           Open the detected project folder"),
        Line::from("  K           Stop the selected Running Now service after confirmation"),
        Line::from(""),
        Line::from(Span::styled(
            "Find exactly what you need",
            bold(ACCENT_COLOR),
        )),
        Line::from("  /           Filter by project, URL, port, stack, process, or command"),
        Line::from("  Tab         Switch between Running Now and Launch Again"),
        Line::from("  a           Toggle smart dev-server detection and every TCP listener"),
        Line::from("  r           Refresh process, port, and project metadata now"),
        Line::from("  ↑↓ / j k    Move selection      g/G  First/last"),
        Line::from(""),
        Line::from(Span::styled("Automatic saving", bold(ACCENT_COLOR))),
        Line::from("  Every detected dev server is saved without a manual action"),
        Line::from("  Stopped servers move into Launch Again automatically"),
        Line::from("  Saved commands always launch from their original project directory"),
        Line::from(""),
        Line::from(Span::styled("Press ? / q / Esc to close help", MUTED_COLOR)),
    ];
    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(ACCENT_COLOR)
                    .title(" JustPorts shortcuts "),
            )
            .wrap(Wrap { trim: true }),
        area,
    );
}

fn render_kill_confirmation(frame: &mut Frame<'_>, app: &App) {
    let Some(server) = app.pending_kill.as_ref() else {
        return;
    };
    let area = centered(frame.area(), 76, 13);
    frame.render_widget(Clear, area);
    let lines = vec![
        Line::from(Span::styled(
            format!("Stop {}?", server.project_name),
            bold(Color::Red),
        )),
        Line::from(""),
        Line::from(format!("  {}", server.url)),
        Line::from(format!(
            "  {}  ·  PID {}  ·  port {}",
            server.process_name, server.pid, server.port
        )),
        Line::from(""),
        Line::from("PID, start time, user ownership, and port ownership will be rechecked."),
        Line::from("Only the selected listener process is stopped. Other services are untouched."),
        Line::from(""),
        Line::from(vec![
            Span::styled(" y / Enter ", bold(Color::Red)),
            Span::raw("stop service     "),
            Span::styled(" n / Esc ", bold(DEV_COLOR)),
            Span::raw("cancel"),
        ]),
    ];
    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Color::Red)
                    .title(" Confirm stop "),
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

fn bold(color: Color) -> Style {
    Style::default().fg(color).add_modifier(Modifier::BOLD)
}

fn highlight_style(focused: bool) -> Style {
    if focused {
        Style::default()
            .bg(SELECTED_BG)
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(MUTED_COLOR)
    }
}

fn format_last_seen(epoch: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let age = now.saturating_sub(epoch);
    if age < 60 {
        "just now".into()
    } else if age < 3_600 {
        format!("{}m ago", age / 60)
    } else if age < 86_400 {
        format!("{}h ago", age / 3_600)
    } else {
        format!("{}d ago", age / 86_400)
    }
}

fn format_age(seconds: u64) -> String {
    if seconds >= 86_400 {
        format!("{}d {}h", seconds / 86_400, (seconds % 86_400) / 3_600)
    } else if seconds >= 3_600 {
        format!("{}h {}m", seconds / 3_600, (seconds % 3_600) / 60)
    } else if seconds >= 60 {
        format!("{}m {}s", seconds / 60, seconds % 60)
    } else {
        format!("{seconds}s")
    }
}

fn format_bytes(bytes: u64) -> String {
    if bytes >= 1_073_741_824 {
        format!("{:.1} GB", bytes as f64 / 1_073_741_824.0)
    } else if bytes >= 1_048_576 {
        format!("{:.0} MB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1_024 {
        format!("{:.0} KB", bytes as f64 / 1_024.0)
    } else {
        format!("{bytes} B")
    }
}
