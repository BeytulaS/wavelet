use std::time::{Duration, Instant};

use ratatui::Frame;
use ratatui::buffer::CellDiffOption;
use ratatui::layout::{Alignment, Constraint, Layout, Margin, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};

use crate::domain::{ItemKey, Playable, Station};
use crate::player::api::{ClockView, Level, Now, RadioStatus, StationView, TrackView};
use crate::tui::browse::{Fetch, Row, Tab};
use crate::tui::input::{Action, PREFIX_POPUP_AFTER};
use crate::tui::paint::components::{gradient_line, gradient_pill, gradient_progress, left_bar_block, pill};
use crate::tui::paint::gradient;
use crate::tui::paint::theme::Theme;
use crate::tui::paint::visualizer::NUM_BANDS;
use crate::tui::{LayoutKind, Mode, Ui};

#[derive(Default, Debug, Clone)]
pub struct FrameOut {
    /// Rows the active list could show; drives half-page motions.
    pub list_rows: usize,
    pub list_offset: usize,
}

const SPINNER: [&str; 8] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧"];

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() > max {
        s.chars().take(max.saturating_sub(1)).collect::<String>() + "…"
    } else {
        s.to_string()
    }
}

fn pad(s: &str, w: usize) -> String {
    let t = truncate(s, w);
    let n = t.chars().count();
    t + &" ".repeat(w.saturating_sub(n))
}

fn fmt_dur(d: Duration) -> String {
    let s = d.as_secs();
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

fn scroll_offset(offset: usize, selected: usize, cap: usize, total: usize, margin: usize) -> usize {
    if cap == 0 || total <= cap {
        return 0;
    }
    let margin = margin.min((cap - 1) / 2);
    offset.min(selected.saturating_sub(margin)).max((selected + margin + 1).saturating_sub(cap)).min(total - cap)
}

pub fn clock_now(c: &ClockView, ui: &Ui, now: Instant) -> Duration {
    if c.running { c.pos + now.saturating_duration_since(ui.received) } else { c.pos }
}

fn hold_area(f: &mut Frame, area: Rect) {
    let buf = f.buffer_mut();
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_diff_option(CellDiffOption::Skip);
            }
        }
    }
}

fn force_area(f: &mut Frame, area: Rect) {
    let buf = f.buffer_mut();
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if let Some(cell) = buf.cell_mut((x, y))
                && cell.diff_option != CellDiffOption::Skip
            {
                cell.set_diff_option(CellDiffOption::AlwaysUpdate);
            }
        }
    }
}

fn artists(p: &Playable) -> String {
    match p {
        Playable::Track(t) => t.artists.iter().map(|a| a.name.as_str()).collect::<Vec<_>>().join(", "),
        Playable::Station(s) => s.name.clone(),
    }
}

fn station_meta(s: &Station) -> String {
    let mut parts: Vec<String> = s.country_code.iter().cloned().collect();
    let codec = match (&s.codec, s.bitrate_kbps) {
        (Some(c), Some(b)) => format!("{c} {b}k"),
        (Some(c), None) => c.clone(),
        (None, Some(b)) => format!("{b}k"),
        (None, None) => String::new(),
    };
    if !codec.is_empty() {
        parts.push(codec);
    }
    parts.join(" · ")
}

fn row_subtitle(item: &Playable) -> String {
    match item {
        Playable::Track(t) => format!("{}  {}", artists(item), t.duration.map(fmt_dur).unwrap_or_default()),
        Playable::Station(s) => station_meta(s),
    }
}

fn now_key(ui: &Ui) -> Option<ItemKey> {
    ui.snap.now.as_ref().map(|n| match n {
        Now::Track { track, .. } => ItemKey::Video(track.id.clone()),
        Now::Station { station, .. } => ItemKey::Station(station.id.clone()),
    })
}

fn is_playing_row(ui: &Ui, row: &Row) -> bool {
    match (row.entry, &ui.snap.now) {
        (Some(id), Some(Now::Track { entry, .. } | Now::Station { entry, .. })) => id == *entry,
        (Some(id), None) => ui.snap.cursor.and_then(|c| ui.snap.queue.get(c)).is_some_and(|r| r.id == id),
        (None, _) => now_key(ui).is_some_and(|k| k == row.item.key()),
    }
}

fn wordmark<'a>(theme: Theme) -> Vec<Span<'a>> {
    gradient_line("ＷＡＶＥＬＥＴ", &[theme.primary, theme.accent])
        .into_iter()
        .map(|mut sp| {
            sp.style = sp.style.add_modifier(Modifier::BOLD);
            sp
        })
        .collect()
}

fn art_size(ui: &Ui, max_w: u16, max_h: u16) -> (u16, u16) {
    let font = ui.art.picker.font_size();
    let fw = font.width.max(1) as u32;
    let fh = font.height.max(1) as u32;
    let mut h = max_h;
    let mut w = (h as u32 * fh / fw) as u16;
    if w > max_w {
        w = max_w;
        h = (w as u32 * fw / fh) as u16;
    }
    (w, h)
}

fn draw_art(f: &mut Frame, ui: &Ui, theme: Theme, rect: Rect) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    match ui.art.cover() {
        Some(c) if ui.force_repaint || c.needs_send(rect) => c.render(f, rect),
        Some(_) => hold_area(f, rect),
        None => {
            f.render_widget(Block::default().style(Style::default().bg(theme.background_element.into())), rect);
            let glyph = if matches!(ui.snap.now, Some(Now::Station { .. })) { "((•))" } else { "♪" };
            let mid = Rect { y: rect.y + rect.height / 2, height: 1, ..rect };
            f.render_widget(
                Paragraph::new(Span::styled(glyph, Style::default().fg(theme.border.into())))
                    .alignment(Alignment::Center),
                mid,
            );
        }
    }
}

// Music loses about 3 dB per octave; the tilt evens that out across the log axis.
const TREBLE_TILT_DB: f32 = 24.0;
const WINDOW_DB: f32 = 30.0;

fn render_spectrum(f: &mut Frame, ui: &Ui, theme: Theme, vrect: Rect) {
    if !ui.spectrum.enabled || vrect.width == 0 || vrect.height == 0 {
        return;
    }
    let Ok(guard) = ui.spectrum.bands.try_lock() else { return };
    if !guard.is_active {
        return;
    }
    let values: [f32; NUM_BANDS] = guard.values;
    let peak = guard.peak_envelope.max(1e-6);
    drop(guard);
    let w = vrect.width as usize;
    let h = vrect.height as usize;
    let mut cols = vec![0.0f32; w];
    for (x, c) in cols.iter_mut().enumerate() {
        let lo = x * NUM_BANDS / w;
        let hi = (((x + 1) * NUM_BANDS / w).max(lo + 1)).min(NUM_BANDS);
        let v = values[lo..hi].iter().sum::<f32>() / (hi - lo) as f32;
        let db = 20.0 * (v / peak).max(1e-6).log10() + TREBLE_TILT_DB * x as f32 / w as f32;
        *c = ((db + WINDOW_DB) / WINDOW_DB).clamp(0.0, 1.0);
    }
    for _ in 0..2 {
        let src = cols.clone();
        for x in 0..w {
            let l = src[x.saturating_sub(1)];
            let r = src[(x + 1).min(w - 1)];
            cols[x] = l * 0.25 + src[x] * 0.5 + r * 0.25;
        }
    }
    const LEVELS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let stops = [theme.info, theme.primary, theme.accent];
    let lines: Vec<Line> = (0..h)
        .map(|row| {
            let from_bottom = (h - 1 - row) as f32;
            let vfrac = if h > 1 { from_bottom / (h - 1) as f32 } else { 0.0 };
            let color: ratatui::style::Color = gradient::interpolate(&stops, vfrac).into();
            Line::from(
                cols.iter()
                    .map(|&v| {
                        let filled = v * h as f32 - from_bottom;
                        if filled >= 1.0 {
                            Span::styled("█", Style::default().fg(color))
                        } else if filled <= 0.0 {
                            Span::raw(" ")
                        } else {
                            let ch = LEVELS[((filled * 8.0) as usize).clamp(1, 8) - 1];
                            Span::styled(ch.to_string(), Style::default().fg(color))
                        }
                    })
                    .collect::<Vec<_>>(),
            )
        })
        .collect();
    f.render_widget(Paragraph::new(lines), vrect);
}

fn now_lines(ui: &Ui, theme: Theme, width: usize) -> Vec<Line<'static>> {
    let (title, artist, meta) = match &ui.snap.now {
        None => {
            let hint = match ui.snap.cursor {
                Some(_) => "space to resume",
                None => "/ to search, ⏎ to play",
            };
            return vec![
                Line::from(Span::styled("Nothing playing", theme.muted())),
                Line::from(Span::styled(hint, theme.muted())),
            ];
        }
        Some(Now::Track { track, .. }) => {
            (track.title.clone(), artists(&Playable::Track(track.clone())), track.album.clone().unwrap_or_default())
        }
        Some(Now::Station { station, stage, .. }) => match stage {
            StationView::OnAir { icy: Some(icy), .. } => (icy.clone(), station.name.clone(), station_meta(station)),
            _ => (station.name.clone(), "no song title from this station".into(), station_meta(station)),
        },
    };
    vec![
        Line::from(Span::styled(
            truncate(&title, width),
            Style::default().fg(theme.text.into()).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(truncate(&artist, width), Style::default().fg(theme.primary.into()))),
        Line::from(Span::styled(truncate(&meta, width), theme.muted())),
    ]
}

fn list_head(ui: &Ui, theme: Theme) -> Line<'static> {
    if let Mode::Search { draft } = &ui.mode {
        return Line::from(vec![
            Span::styled("search: ", theme.heading()),
            Span::styled(format!("{draft}▏"), Style::default().fg(theme.text.into())),
        ]);
    }
    let tab = ui.browse.tab;
    let mut spans = vec![Span::styled(tab.label().to_string(), theme.heading())];
    if let Some(list) = ui.browse.list(tab) {
        if !list.query.is_empty() {
            spans.push(Span::styled(format!("  “{}”", list.query), Style::default().fg(theme.text.into())));
        }
        spans.push(Span::styled(format!("  {}", list.items.len()), theme.muted()));
        match &list.state {
            Fetch::Loading(_) => spans.push(Span::styled(format!("  {}", SPINNER[ui.spin % 8]), theme.heading())),
            Fetch::Failed(e) => spans.push(Span::styled(format!("  failed: {}", truncate(e, 40)), theme.muted())),
            Fetch::Idle => {}
        }
    } else {
        spans.push(Span::styled(format!("  {}", ui.browse.rows(&ui.snap).len()), theme.muted()));
        if tab == Tab::Queue
            && let RadioStatus::Following { fetching, dry, .. } = &ui.snap.radio
        {
            let s = if *fetching {
                "  radio: fetching…"
            } else if *dry {
                "  radio: no more"
            } else {
                "  radio on"
            };
            spans.push(Span::styled(s, theme.muted()));
        }
    }
    Line::from(spans)
}

fn empty_hint(ui: &Ui) -> &'static str {
    match ui.browse.tab {
        Tab::YouTube | Tab::Radio => {
            let loading = ui.browse.list(ui.browse.tab).is_some_and(|l| matches!(l.state, Fetch::Loading(_)));
            if loading { "searching…" } else { "Press / to search" }
        }
        Tab::Favorites => "Press f on a song or station to keep it here",
        Tab::Queue => "The queue is empty",
    }
}

fn render_list_compact(f: &mut Frame, ui: &Ui, theme: Theme, area: Rect, out: &mut FrameOut) {
    f.render_widget(Block::default().style(theme.panel()), area);
    let inner = area.inner(Margin::new(2, 1));
    if inner.height < 3 {
        return;
    }
    f.render_widget(Paragraph::new(list_head(ui, theme)), Rect { height: 1, ..inner });
    let top = inner.y + 2;
    let cap = (inner.bottom() - top) as usize;
    let rows = ui.browse.rows(&ui.snap);
    out.list_rows = cap;
    if rows.is_empty() {
        f.render_widget(
            Paragraph::new(Span::styled(empty_hint(ui), theme.muted())),
            Rect { y: top, height: 1, ..inner },
        );
        return;
    }
    let cur = ui.browse.cursor();
    let selected = cur.selected.min(rows.len() - 1);
    let off = scroll_offset(cur.offset, selected, cap, rows.len(), 3);
    out.list_offset = off;
    for (i, row) in rows.iter().enumerate().skip(off).take(cap) {
        let rect = Rect { y: top + (i - off) as u16, height: 1, ..inner };
        let sel = i == selected;
        let bg = if sel { theme.element_bg() } else { theme.panel_bg() };
        let style = Style::default().fg(theme.text.into());
        let style = if sel { style.add_modifier(Modifier::BOLD) } else { style };
        let w = inner.width.saturating_sub(2) as usize;
        let sub_w = w * 2 / 5;
        let marker = if is_playing_row(ui, row) {
            " ●"
        } else if row.from_radio {
            " ·"
        } else {
            ""
        };
        let fav = if ui.snap.favorites.contains(&row.item.key()) { " ♥" } else { "" };
        let title_w = w.saturating_sub(sub_w + 2 + marker.chars().count() + fav.chars().count());
        let mut spans = vec![];
        if !marker.is_empty() {
            spans.push(Span::styled(marker, Style::default().fg(theme.accent.into()).add_modifier(Modifier::BOLD)));
        }
        spans.push(Span::styled(format!(" {}", pad(row.item.title(), title_w)), style));
        if !fav.is_empty() {
            spans.push(Span::styled(fav, Style::default().fg(theme.error.into())));
        }
        let sub = truncate(&row_subtitle(row.item), sub_w);
        spans.push(Span::styled(format!(" {sub:>sub_w$}"), theme.muted()));
        f.render_widget(Paragraph::new(Line::from(spans)).block(left_bar_block(&theme, sel, bg)), rect);
    }
}

fn render_list_table(f: &mut Frame, ui: &Ui, theme: Theme, area: Rect, out: &mut FrameOut) {
    f.render_widget(Block::default().style(theme.panel()), area);
    let inner = area.inner(Margin::new(2, 1));
    if inner.height < 4 {
        return;
    }
    f.render_widget(Paragraph::new(list_head(ui, theme)), Rect { height: 1, ..inner });
    let rows = ui.browse.rows(&ui.snap);
    let stations = rows.first().is_some_and(|r| matches!(r.item, Playable::Station(_)));
    let w = inner.width.saturating_sub(4) as usize;
    let (cols, header): (Vec<usize>, Vec<&str>) = if stations {
        let rest = w.saturating_sub(4 + 6 + 6 + 3);
        (vec![rest * 55 / 100, rest - rest * 55 / 100, 4, 6, 6], vec!["STATION", "TAGS", "CC", "CODEC", "KBPS"])
    } else {
        let d = 6;
        let rest = w.saturating_sub(d + 3);
        let (a, b) = (rest * 45 / 100, rest * 28 / 100);
        (vec![a, b, rest - a - b, d], vec!["TITLE", "ARTIST", "ALBUM", "TIME"])
    };
    let header_line: String = header.iter().zip(&cols).map(|(h, w)| pad(h, *w) + " ").collect();
    f.render_widget(
        Paragraph::new(Span::styled(format!("   {header_line}"), Style::default().fg(theme.border.into()))),
        Rect { y: inner.y + 2, height: 1, ..inner },
    );
    let top = inner.y + 3;
    let cap = (inner.bottom() - top) as usize;
    out.list_rows = cap;
    if rows.is_empty() {
        f.render_widget(
            Paragraph::new(Span::styled(empty_hint(ui), theme.muted())),
            Rect { y: top, height: 1, ..inner },
        );
        return;
    }
    let cur = ui.browse.cursor();
    let selected = cur.selected.min(rows.len() - 1);
    let off = scroll_offset(cur.offset, selected, cap, rows.len(), 3);
    out.list_offset = off;
    for (i, row) in rows.iter().enumerate().skip(off).take(cap) {
        let rect = Rect { y: top + (i - off) as u16, height: 1, ..inner };
        let sel = i == selected;
        let bg = if sel { theme.element_bg() } else { theme.panel_bg() };
        let fields: Vec<String> = match row.item {
            Playable::Track(t) => vec![
                t.title.clone(),
                artists(row.item),
                t.album.clone().unwrap_or_default(),
                t.duration.map(fmt_dur).unwrap_or_default(),
            ],
            Playable::Station(s) => vec![
                s.name.clone(),
                s.tags.join(" "),
                s.country_code.clone().unwrap_or_default(),
                s.codec.clone().unwrap_or_default(),
                s.bitrate_kbps.map(|b| b.to_string()).unwrap_or_default(),
            ],
        };
        let playing = is_playing_row(ui, row);
        let mut spans = vec![Span::styled(
            if playing { " ● " } else { "   " },
            Style::default().fg(theme.accent.into()).add_modifier(Modifier::BOLD),
        )];
        for (j, (fv, cw)) in fields.iter().zip(&cols).enumerate() {
            let st = if j == 0 {
                let s = Style::default().fg(if playing { theme.primary } else { theme.text }.into());
                if sel { s.add_modifier(Modifier::BOLD) } else { s }
            } else {
                theme.muted()
            };
            spans.push(Span::styled(pad(fv, *cw) + " ", st));
        }
        f.render_widget(Paragraph::new(Line::from(spans)).block(left_bar_block(&theme, sel, bg)), rect);
    }
}

fn render_header(f: &mut Frame, ui: &Ui, theme: Theme, area: Rect) {
    let mut spans = wordmark(theme);
    spans.push(Span::raw("   "));
    for (tab, label) in [
        (Tab::YouTube, "1 YouTube"),
        (Tab::Radio, "2 Radio"),
        (Tab::Favorites, "3 Favorites"),
        (Tab::Queue, "gq Queue"),
    ] {
        if ui.browse.tab == tab {
            spans.extend(gradient_pill(label, &[theme.primary, theme.accent], theme.background));
        } else {
            spans.push(Span::styled(format!(" {label} "), theme.muted()));
        }
        spans.push(Span::raw(" "));
    }
    if let Some(c) = ui.snap.cursor {
        let up = ui.snap.queue.len().saturating_sub(c + 1);
        spans.push(Span::styled(format!("  {up} up next"), theme.muted()));
    }
    if let RadioStatus::Following { fetching: true, .. } = ui.snap.radio {
        spans.push(Span::styled("  radio…", theme.muted()));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
    let key = |k: &'static str| Span::styled(k, Style::default().fg(theme.primary.into()));
    let val = |v: &str| Span::styled(v.to_string(), Style::default().fg(theme.text.into()));
    let right = Line::from(vec![
        key("w "),
        val(ui.layout.label()),
        Span::styled("  ·  ", theme.muted()),
        key("t "),
        val(ui.look.mode.label()),
        Span::styled("  ·  ", theme.muted()),
        key("v "),
        val(if ui.spectrum.enabled { "viz on" } else { "viz off" }),
    ]);
    f.render_widget(Paragraph::new(right).alignment(Alignment::Right), area);
}

fn render_strip(f: &mut Frame, ui: &Ui, theme: Theme, area: Rect, now: Instant) {
    let rows = Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).split(area);
    let spin = SPINNER[ui.spin % 8];
    let mut left = vec![];
    let (glyph, title, artist, status) = match &ui.snap.now {
        None => {
            ("■".to_string(), String::new(), String::new(), if ui.snap.cursor.is_some() { "stopped" } else { "idle" })
        }
        Some(Now::Track { track, stage, .. }) => {
            let (g, s) = match stage {
                TrackView::Resolving => (spin.to_string(), "resolving stream…"),
                TrackView::Loading => (spin.to_string(), "buffering…"),
                TrackView::Playing { clock, .. } if clock.running => ("▶".into(), ""),
                TrackView::Playing { .. } => ("⏸".into(), ""),
            };
            (g, track.title.clone(), artists(&Playable::Track(track.clone())), s)
        }
        Some(Now::Station { station, stage, .. }) => {
            let (g, icy, s) = match stage {
                StationView::Connecting => (spin.to_string(), None, "connecting…"),
                StationView::OnAir { listened, icy } => {
                    (if listened.running { "▶" } else { "⏸" }.to_string(), icy.clone(), "")
                }
            };
            (g, icy.unwrap_or_else(|| "—".into()), station.name.clone(), s)
        }
    };
    left.push(Span::styled(format!("{glyph} "), theme.heading()));
    if title.is_empty() {
        left.push(Span::styled(status, theme.muted()));
    } else {
        left.push(Span::styled(
            truncate(&title, 50),
            Style::default().fg(theme.text.into()).add_modifier(Modifier::BOLD),
        ));
        left.push(Span::styled("  ", theme.muted()));
        left.push(Span::styled(truncate(&artist, 40), Style::default().fg(theme.primary.into())));
        if !status.is_empty() {
            left.push(Span::styled(format!("  {status}"), theme.muted()));
        }
    }
    f.render_widget(Paragraph::new(Line::from(left)), rows[0]);
    const VLEV: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let vol = ui.snap.volume.get() as usize;
    let filled = (vol.min(100) * VLEV.len() + 50) / 100;
    let mut vspans: Vec<Span> = VLEV
        .iter()
        .enumerate()
        .map(|(i, ch)| {
            let c = if i < filled { theme.primary } else { theme.border_dimmest };
            Span::styled(ch.to_string(), Style::default().fg(c.into()))
        })
        .collect();
    vspans.push(Span::styled(format!(" {vol:>3}%"), theme.muted()));
    f.render_widget(Paragraph::new(Line::from(vspans)).alignment(Alignment::Right), rows[0]);

    if let Some(Now::Station { stage, .. }) = &ui.snap.now {
        let listened = match stage {
            StationView::OnAir { listened, .. } => clock_now(listened, ui, now),
            StationView::Connecting => Duration::ZERO,
        };
        let mut spans = pill("LIVE", theme.error, theme.background);
        spans.push(Span::styled(format!("  on air {}", fmt_dur(listened)), theme.muted()));
        let rest = (rows[1].width as usize).saturating_sub(20);
        spans.push(Span::raw(" "));
        let line = gradient_progress(
            rest,
            rest,
            &[theme.border_dimmest, theme.primary, theme.border_dimmest],
            theme.border_dimmest,
        );
        spans.extend(line.into_iter().map(|mut s| {
            s.content = "─".into();
            s
        }));
        f.render_widget(Paragraph::new(Line::from(spans)), rows[1]);
    } else {
        let (pos, dur) = match &ui.snap.now {
            Some(Now::Track { stage: TrackView::Playing { clock, duration }, track, .. }) => {
                let d = duration.or(track.duration).unwrap_or_default();
                let p = clock_now(clock, ui, now);
                (if d.is_zero() { p } else { p.min(d) }, d)
            }
            Some(Now::Track { track, .. }) => (Duration::ZERO, track.duration.unwrap_or_default()),
            _ => (Duration::ZERO, Duration::ZERO),
        };
        let l = format!("{} ", fmt_dur(pos));
        let r = format!(" {}", fmt_dur(dur));
        let bar_w = (rows[1].width as usize).saturating_sub(l.chars().count() + r.chars().count());
        let filled = if dur.is_zero() { 0 } else { (pos.as_secs_f64() / dur.as_secs_f64() * bar_w as f64) as usize };
        let mut spans = vec![Span::styled(l, theme.muted())];
        spans.extend(gradient_progress(bar_w, filled.min(bar_w), &[theme.primary, theme.accent], theme.border_dimmest));
        spans.push(Span::styled(r, theme.muted()));
        f.render_widget(Paragraph::new(Line::from(spans)), rows[1]);
    }
}

fn render_footer(f: &mut Frame, theme: Theme, area: Rect) {
    let key = |k: &'static str| Span::styled(k, Style::default().fg(theme.primary.into()));
    let lbl = |t: &'static str| Span::styled(t, theme.muted());
    let hints = [
        ("⇥", " source   "),
        ("/", " search   "),
        ("⏎", " play   "),
        ("␣", " pause   "),
        ("n/b", " next/prev   "),
        ("a", " play next   "),
        ("f", " fav   "),
        ("gq", " queue   "),
        ("w", " layout   "),
        ("t", " theme   "),
        ("?", " help   "),
        ("q", " close   "),
        ("Q", " quit"),
    ];
    let line = Line::from(hints.into_iter().flat_map(|(k, l)| [key(k), lbl(l)]).collect::<Vec<_>>());
    f.render_widget(Paragraph::new(line).alignment(Alignment::Center), area);
}

fn popup(area: Rect, w: u16, h: u16) -> Rect {
    let (w, h) = (w.min(area.width), h.min(area.height));
    Rect {
        x: area.x + area.width.saturating_sub(w) / 2,
        y: area.y + area.height.saturating_sub(h) / 2,
        width: w,
        height: h,
    }
}

fn render_help(f: &mut Frame, ui: &Ui, theme: Theme, area: Rect) {
    let all = ui.keymap.bindings_starting_with(&[]);
    let mut lines = vec![Line::from(Span::styled("Keys", theme.heading())), Line::raw("")];
    for action in Action::ALL.iter().filter(|a| **a != Action::Unbind) {
        let keys: Vec<&str> = all.iter().filter(|(_, a)| a == action).map(|(k, _)| k.as_str()).collect();
        if keys.is_empty() {
            continue;
        }
        lines.push(Line::from(vec![
            Span::styled(pad(&keys.join(" · "), 18), Style::default().fg(theme.primary.into())),
            Span::styled(action.help(), Style::default().fg(theme.text.into())),
        ]));
    }
    let rect = popup(area, 62, lines.len() as u16 + 2);
    f.render_widget(Clear, rect);
    f.render_widget(Block::default().style(theme.element()), rect);
    f.render_widget(Paragraph::new(lines), rect.inner(Margin::new(2, 1)));
    force_area(f, rect);
}

fn render_prefix(f: &mut Frame, ui: &Ui, theme: Theme, area: Rect, now: Instant) {
    let Some((prefix, since)) = ui.keys.pending() else { return };
    if now.saturating_duration_since(since) < PREFIX_POPUP_AFTER {
        return;
    }
    let lines: Vec<Line> = ui
        .keymap
        .bindings_starting_with(prefix)
        .into_iter()
        .map(|(k, a)| {
            Line::from(vec![
                Span::styled(pad(&k, 6), Style::default().fg(theme.primary.into())),
                Span::styled(a.help(), theme.muted()),
            ])
        })
        .collect();
    let h = lines.len() as u16 + 2;
    let rect = Rect { x: area.x, y: area.bottom().saturating_sub(h + 3), width: 34.min(area.width), height: h };
    f.render_widget(Clear, rect);
    f.render_widget(Block::default().style(theme.element()), rect);
    f.render_widget(Paragraph::new(lines), rect.inner(Margin::new(1, 1)));
    force_area(f, rect);
}

fn render_toasts(f: &mut Frame, ui: &Ui, theme: Theme, area: Rect) {
    let items: Vec<_> = ui.toasts.visible().collect();
    let mut y = area.bottom().saturating_sub(4);
    for n in items.iter().rev() {
        let color = match n.level {
            Level::Info => theme.primary,
            Level::Warn => theme.warning,
            Level::Error => theme.error,
        };
        let text = truncate(&n.text, 56);
        let w = text.chars().count() as u16 + 4;
        let rect = Rect { x: area.right().saturating_sub(w), y, width: w.min(area.width), height: 1 };
        f.render_widget(Clear, rect);
        let line = Line::from(vec![
            Span::styled("▍", Style::default().fg(color.into()).bg(theme.background_element.into())),
            Span::styled(format!(" {text}  "), theme.element()),
        ]);
        f.render_widget(Paragraph::new(line), rect);
        force_area(f, rect);
        y = y.saturating_sub(1);
    }
}

pub fn frame(f: &mut Frame, ui: &Ui, now: Instant) -> FrameOut {
    let mut out = FrameOut::default();
    let theme = ui.look.theme();
    let full = f.area();
    f.render_widget(Block::default().style(theme.base()), full);
    let area = full.inner(Margin::new(2, 1));
    let thin_viz = ui.layout == LayoutKind::List && ui.spectrum.enabled;
    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(6),
        Constraint::Length(if thin_viz { 4 } else { 1 }),
        Constraint::Length(2),
        Constraint::Length(1),
    ])
    .split(area);
    render_header(f, ui, theme, rows[0]);
    match ui.layout {
        LayoutKind::Myx => layout_myx(f, ui, theme, rows[2], &mut out),
        LayoutKind::Stage => layout_stage(f, ui, theme, rows[2], &mut out),
        LayoutKind::List => layout_list(f, ui, theme, rows[2], &mut out),
    }
    if thin_viz {
        let r = rows[3];
        render_spectrum(f, ui, theme, Rect { y: r.y, height: 3, ..r });
    }
    render_strip(f, ui, theme, rows[4], now);
    render_footer(f, theme, rows[5]);
    render_toasts(f, ui, theme, area);
    render_prefix(f, ui, theme, area, now);
    if ui.mode == Mode::Help {
        render_help(f, ui, theme, area);
    }
    if ui.force_repaint {
        force_area(f, full);
    }
    out
}

fn layout_myx(f: &mut Frame, ui: &Ui, theme: Theme, body: Rect, out: &mut FrameOut) {
    let cols = Layout::horizontal([Constraint::Percentage(32), Constraint::Min(24)]).spacing(3).split(body);
    render_list_compact(f, ui, theme, cols[0], out);
    let right = cols[1];
    let viz_h = if ui.spectrum.enabled { 9 } else { 0 };
    let chunks = Layout::vertical([Constraint::Min(6), Constraint::Length(viz_h)]).split(right);
    let top = chunks[0];
    let top = Rect { y: top.y + 1, height: top.height.saturating_sub(1), ..top };
    let (aw, ah) = art_size(ui, top.width, top.height.saturating_sub(4).clamp(3, 16));
    let group_h = ah + 4;
    let art = Rect {
        x: top.x + top.width.saturating_sub(aw) / 2,
        y: top.y + top.height.saturating_sub(group_h) / 2,
        width: aw,
        height: ah,
    };
    draw_art(f, ui, theme, art);
    let text = Rect { y: art.y + ah + 1, height: 3, ..top };
    f.render_widget(Paragraph::new(now_lines(ui, theme, top.width as usize)).alignment(Alignment::Center), text);
    let viz = chunks[1];
    let vw = ((viz.width as u32 * 9 / 10) as u16).clamp(24, 80).min(viz.width);
    let vh = viz.height.saturating_sub(2).min(7);
    render_spectrum(
        f,
        ui,
        theme,
        Rect { x: viz.x + viz.width.saturating_sub(vw) / 2, y: viz.y, width: vw, height: vh },
    );
}

fn layout_stage(f: &mut Frame, ui: &Ui, theme: Theme, body: Rect, out: &mut FrameOut) {
    let parts = Layout::vertical([Constraint::Min(8), Constraint::Length(1), Constraint::Length(11)]).split(body);
    let stage = parts[0];
    let (aw, ah) = art_size(ui, stage.width / 2, stage.height);
    let art = Rect { x: stage.x + 2, y: stage.y + stage.height.saturating_sub(ah) / 2, width: aw, height: ah };
    draw_art(f, ui, theme, art);
    let rx = art.right() + 5;
    let right = Rect { x: rx, y: stage.y, width: stage.right().saturating_sub(rx + 1), height: stage.height };
    let rows =
        Layout::vertical([Constraint::Length(1), Constraint::Length(8), Constraint::Min(3), Constraint::Length(1)])
            .split(right);
    let w = right.width as usize;
    let nl = now_lines(ui, theme, w);
    let lines: Vec<Line> = match &ui.snap.now {
        Some(n) => {
            let (src, extra, live) = match n {
                Now::Track { track, .. } => ("YOUTUBE MUSIC", track.duration.map(fmt_dur).unwrap_or_default(), false),
                Now::Station { station, .. } => ("INTERNET RADIO", station_meta(station), true),
            };
            let mut l1 = gradient_pill(src, &[theme.primary, theme.accent], theme.background);
            if live {
                l1.push(Span::raw(" "));
                l1.extend(pill("LIVE", theme.error, theme.background));
            }
            l1.push(Span::styled(format!("   {extra}"), theme.muted()));
            let title = nl[0].spans.first().map(|s| s.content.to_string()).unwrap_or_default();
            let big: Vec<Span> = gradient_line(&title, &[theme.text, theme.primary])
                .into_iter()
                .map(|mut s| {
                    s.style = s.style.add_modifier(Modifier::BOLD);
                    s
                })
                .collect();
            vec![Line::from(l1), Line::raw(""), Line::from(big), Line::raw(""), nl[1].clone(), nl[2].clone()]
        }
        None => nl,
    };
    f.render_widget(Paragraph::new(lines), rows[1]);
    render_spectrum(f, ui, theme, Rect { height: rows[2].height.min(16), ..rows[2] });
    render_list_compact(f, ui, theme, parts[2], out);
}

fn layout_list(f: &mut Frame, ui: &Ui, theme: Theme, body: Rect, out: &mut FrameOut) {
    let cols = Layout::horizontal([Constraint::Min(40), Constraint::Length(34)]).spacing(2).split(body);
    render_list_table(f, ui, theme, cols[0], out);
    let rail = cols[1];
    f.render_widget(Block::default().style(theme.panel()), rail);
    let inner = rail.inner(Margin::new(2, 1));
    let (aw, ah) = art_size(ui, inner.width, 12.min(inner.height / 2));
    let art = Rect { x: inner.x + inner.width.saturating_sub(aw) / 2, y: inner.y, width: aw, height: ah };
    draw_art(f, ui, theme, art);
    let mut lines = now_lines(ui, theme, inner.width as usize);
    lines.insert(0, Line::raw(""));
    if let Some(c) = ui.snap.cursor {
        lines.push(Line::raw(""));
        lines.push(Line::from(Span::styled(
            "UP NEXT",
            Style::default().fg(theme.border.into()).add_modifier(Modifier::BOLD),
        )));
        for r in ui.snap.queue.iter().skip(c + 1).take(6) {
            lines.push(Line::from(vec![
                Span::styled("· ", Style::default().fg(theme.accent.into())),
                Span::styled(truncate(r.item.title(), (inner.width as usize).saturating_sub(2)), theme.muted()),
            ]));
        }
    }
    let text = Rect { y: art.bottom(), height: inner.bottom().saturating_sub(art.bottom()), ..inner };
    f.render_widget(Paragraph::new(lines), text);
}
