use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, LineGauge, Padding, Paragraph};
use ratatui::Frame;

use sendspin::protocol::messages::RepeatMode;
use sendspin::sync::SyncQuality;

use crate::app::AppState;

/// Render the full TUI layout into the given frame.
pub fn draw(frame: &mut Frame, state: &AppState) {
    let area = frame.area();

    let outer = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if state.connected {
            Color::Green
        } else {
            Color::DarkGray
        }))
        .title(" sendspin ")
        .title_style(Style::default().bold())
        .padding(Padding::new(2, 2, 1, 0));

    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let layout = Layout::vertical([
        Constraint::Length(1), // title
        Constraint::Length(1), // artist - album
        Constraint::Length(1), // spacer
        Constraint::Length(1), // progress bar
        Constraint::Length(1), // spacer
        Constraint::Length(1), // controls
        Constraint::Min(0),    // fill
        Constraint::Length(1), // status bar
    ])
    .split(inner);

    draw_track_info(frame, state, layout[0], layout[1]);
    draw_progress(frame, state, layout[3]);
    draw_controls(frame, state, layout[5]);
    draw_status(frame, state, layout[7]);
}

fn draw_track_info(frame: &mut Frame, state: &AppState, title_area: Rect, artist_area: Rect) {
    let title = state
        .title
        .as_deref()
        .unwrap_or(if state.connected {
            "Waiting for track..."
        } else {
            "Not connected"
        });

    let title_line = Line::from(vec![
        Span::styled(
            if state.is_playing() { "▶ " } else { "⏸ " },
            Style::default().fg(Color::Cyan),
        ),
        Span::styled(title, Style::default().bold().fg(Color::White)),
    ]);
    frame.render_widget(Paragraph::new(title_line), title_area);

    let artist = state.artist.as_deref().unwrap_or("");
    let album = state.album.as_deref().unwrap_or("");
    let subtitle = match (artist.is_empty(), album.is_empty()) {
        (true, true) => String::new(),
        (false, true) => format!("  {artist}"),
        (true, false) => format!("  {album}"),
        (false, false) => format!("  {artist} — {album}"),
    };
    let artist_line = Line::from(Span::styled(
        subtitle,
        Style::default().fg(Color::DarkGray),
    ));
    frame.render_widget(Paragraph::new(artist_line), artist_area);
}

fn draw_progress(frame: &mut Frame, state: &AppState, area: Rect) {
    let current = format_time(state.current_progress_ms());
    let total = format_time(state.track_duration_ms);

    // Split area: time_left | gauge | time_right
    let chunks = Layout::horizontal([
        Constraint::Length(current.len() as u16 + 1),
        Constraint::Min(10),
        Constraint::Length(total.len() as u16 + 1),
    ])
    .split(area);

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!("{current} "),
            Style::default().fg(Color::DarkGray),
        ))),
        chunks[0],
    );

    let gauge = LineGauge::default()
        .ratio(state.progress_ratio())
        .filled_style(Style::default().fg(Color::Cyan))
        .unfilled_style(Style::default().fg(Color::DarkGray))
        .line_set(ratatui::symbols::line::THICK);
    frame.render_widget(gauge, chunks[1]);

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!(" {total}"),
            Style::default().fg(Color::DarkGray),
        ))),
        chunks[2],
    );
}

fn draw_controls(frame: &mut Frame, state: &AppState, area: Rect) {
    let vol_bar = volume_bar(state.volume, state.muted);

    let repeat_str = match state.repeat {
        RepeatMode::Off => "off",
        RepeatMode::One => "one",
        RepeatMode::All => "all",
    };

    let line = Line::from(vec![
        Span::styled("vol ", Style::default().fg(Color::DarkGray)),
        Span::styled(vol_bar, Style::default().fg(if state.muted { Color::Red } else { Color::Cyan })),
        Span::styled(
            format!(" {:>3}%", if state.muted { 0 } else { state.volume }),
            Style::default().fg(Color::DarkGray),
        ),
        Span::raw("    "),
        Span::styled("repeat ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            repeat_str,
            Style::default().fg(if matches!(state.repeat, RepeatMode::Off) {
                Color::DarkGray
            } else {
                Color::Cyan
            }),
        ),
        Span::raw("    "),
        Span::styled("shuffle ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            if state.shuffle { "on" } else { "off" },
            Style::default().fg(if state.shuffle {
                Color::Cyan
            } else {
                Color::DarkGray
            }),
        ),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn draw_status(frame: &mut Frame, state: &AppState, area: Rect) {
    let mut spans = Vec::new();

    // Connection status
    if state.connected {
        spans.push(Span::styled("connected", Style::default().fg(Color::Green)));
    } else {
        spans.push(Span::styled(
            "disconnected",
            Style::default().fg(Color::Red),
        ));
    }

    // Sync info
    if let (Some(rtt), Some(quality)) = (state.sync_rtt_ms, &state.sync_quality) {
        let color = match quality {
            SyncQuality::Good => Color::Green,
            SyncQuality::Degraded => Color::Yellow,
            SyncQuality::Lost => Color::Red,
        };
        spans.push(Span::styled(" │ ", Style::default().fg(Color::DarkGray)));
        spans.push(Span::styled(format!("sync {rtt:.0}ms"), Style::default().fg(color)));
    }

    // Audio format
    if let Some(ref fmt) = state.audio_format {
        spans.push(Span::styled(" │ ", Style::default().fg(Color::DarkGray)));
        spans.push(Span::styled(
            format!("{}Hz/{}bit/{}ch", fmt.sample_rate, fmt.bit_depth, fmt.channels),
            Style::default().fg(Color::DarkGray),
        ));
    }

    // Group name
    if let Some(ref name) = state.group_name {
        spans.push(Span::styled(" │ ", Style::default().fg(Color::DarkGray)));
        spans.push(Span::styled(name.as_str(), Style::default().fg(Color::DarkGray)));
    }

    // Error
    if let Some(ref err) = state.error {
        spans.push(Span::styled(" │ ", Style::default().fg(Color::DarkGray)));
        spans.push(Span::styled(err.as_str(), Style::default().fg(Color::Red)));
    }

    // Keybinds hint (right-aligned would need more layout, keep it simple)
    spans.push(Span::styled(" │ ", Style::default().fg(Color::DarkGray)));
    spans.push(Span::styled(
        "q:quit spc:play n/p:skip ±:vol m:mute r:repeat s:shuffle",
        Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM),
    ));

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn volume_bar(volume: u8, muted: bool) -> String {
    if muted {
        return "──────────".to_string();
    }
    let filled = (volume as usize) / 10;
    let empty = 10 - filled;
    format!("{}{}", "━".repeat(filled), "─".repeat(empty))
}

fn format_time(ms: i64) -> String {
    if ms <= 0 {
        return "0:00".to_string();
    }
    let total_secs = ms / 1000;
    let mins = total_secs / 60;
    let secs = total_secs % 60;
    format!("{mins}:{secs:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_time_zero() {
        assert_eq!(format_time(0), "0:00");
    }

    #[test]
    fn format_time_negative() {
        assert_eq!(format_time(-1000), "0:00");
    }

    #[test]
    fn format_time_seconds() {
        assert_eq!(format_time(5_000), "0:05");
        assert_eq!(format_time(59_000), "0:59");
    }

    #[test]
    fn format_time_minutes() {
        assert_eq!(format_time(60_000), "1:00");
        assert_eq!(format_time(90_000), "1:30");
        assert_eq!(format_time(183_000), "3:03");
    }

    #[test]
    fn format_time_long() {
        assert_eq!(format_time(3_600_000), "60:00");
    }

    #[test]
    fn volume_bar_full() {
        assert_eq!(volume_bar(100, false), "━━━━━━━━━━");
    }

    #[test]
    fn volume_bar_empty() {
        assert_eq!(volume_bar(0, false), "──────────");
    }

    #[test]
    fn volume_bar_half() {
        assert_eq!(volume_bar(50, false), "━━━━━─────");
    }

    #[test]
    fn volume_bar_muted() {
        assert_eq!(volume_bar(100, true), "──────────");
        assert_eq!(volume_bar(50, true), "──────────");
    }
}
