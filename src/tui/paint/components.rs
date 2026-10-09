//! Reusable render primitives in noodle's visual language.
//!
//! These are the building blocks that make the surface look designed:
//!   * `pill` — the solid badge (GET/POST → EXPLICIT).
//!   * `progress` — a played/unplayed bar.
//!   * `left_bar_block` — the signature focus cue: a heavy `┃` on the left edge
//!     instead of a full box.

use crate::tui::paint::gradient::Rgb;
use crate::tui::paint::theme::Theme;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::{Block, BorderType, Borders};

/// A solid pill badge: `bg` fill + one space of horizontal padding either side.
/// Mirrors noodle's `<Badge>`.
pub fn pill<'a>(label: &str, bg: Rgb, fg: Rgb) -> Vec<Span<'a>> {
    vec![Span::styled(
        format!(" {label} "),
        Style::default()
            .bg(bg.into())
            .fg(fg.into())
            .add_modifier(Modifier::BOLD),
    )]
}

/// The signature focus cue. A left-only heavy bar (`┃`) whose color signals
/// focus. Fill the block with `bg` so the row reads as elevated.
///
/// `focused` → `border_active`; otherwise `border_subtle`.
pub fn left_bar_block(theme: &Theme, focused: bool, bg: Color) -> Block<'static> {
    let bar_color: Color = if focused {
        theme.border_active.into()
    } else {
        theme.border_subtle.into()
    };
    Block::default()
        .borders(Borders::LEFT)
        .border_type(BorderType::Thick)
        .border_style(Style::default().fg(bar_color))
        .style(Style::default().bg(bg))
}

/// A horizontal progress bar as a span vector: `filled` cells (0..=width) in
/// `fill`, the rest in `track`.
pub fn progress<'a>(width: usize, filled: usize, fill: Rgb, track: Rgb) -> Vec<Span<'a>> {
    let filled = filled.min(width);
    (0..width)
        .map(|i| Span::styled("▬", Style::default().fg(if i < filled { fill } else { track }.into())))
        .collect()
}
