use ratatui::style::{Color, Modifier, Style};

pub const GOOD: Color = Color::Rgb(111, 214, 151);
pub const ACCENT: Color = Color::Rgb(104, 182, 255);
pub const SECONDARY: Color = Color::Rgb(194, 145, 255);
pub const WARNING: Color = Color::Rgb(245, 184, 90);
pub const MUTED: Color = Color::Rgb(118, 128, 145);
pub const SELECTED_BG: Color = Color::Rgb(36, 48, 64);

pub fn bold(color: Color) -> Style {
    Style::default().fg(color).add_modifier(Modifier::BOLD)
}

pub fn selected() -> Style {
    Style::default()
        .bg(SELECTED_BG)
        .fg(Color::White)
        .add_modifier(Modifier::BOLD)
}
