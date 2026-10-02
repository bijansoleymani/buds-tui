use std::time::Duration;

use maestro::service::settings::{AncState, EqBands};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
};

use crate::app::{App, Battery, EQ_BANDS, ROWS, Row, eq_band};

const ACCENT: Color = Color::Cyan;
const FOCUS: Color = Color::Green;
const HEADER: Color = Color::Yellow;
const FG: Color = Color::White;
const DIM: Color = Color::DarkGray;

const WIDTH: u16 = 64;
const MESSAGE_TTL: Duration = Duration::from_secs(4);

pub fn draw(f: &mut Frame, app: &App) {
    draw_in(f, f.area(), app);
}

/// Draws into `area` only, so a host can put something above or around it.
pub fn draw_in(f: &mut Frame, area: Rect, app: &App) {
    let col = centered_col(area, WIDTH);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // title + status
            Constraint::Length(5), // battery
            Constraint::Max(17),   // settings
            Constraint::Length(1), // footer
            Constraint::Fill(1),
        ])
        .split(col);

    draw_header(f, chunks[0], app);
    draw_battery(f, chunks[1], app);
    draw_settings(f, chunks[2], app);
    draw_footer(f, chunks[3], app);

    if app.show_info {
        draw_info(f, area, app);
    }
}

fn draw_header(f: &mut Frame, area: Rect, app: &App) {
    let name = app
        .device
        .as_ref()
        .map(|(n, _)| n.as_str())
        .unwrap_or("Pixel Buds");
    let (dot, dot_color) = if app.connected { ("●", FOCUS) } else { ("○", DIM) };

    let title = Line::from(vec![
        Span::styled(name, Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        Span::styled(dot, Style::default().fg(dot_color)),
    ]);
    let status = Line::styled(app.status.as_str(), Style::default().fg(DIM));
    f.render_widget(
        Paragraph::new(vec![title, status]).alignment(Alignment::Center),
        area,
    );
}

fn draw_battery(f: &mut Frame, area: Rect, app: &App) {
    let block = section("Battery", false);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let placement = |in_case: Option<bool>| match in_case {
        Some(true) => Some("in case"),
        _ => None,
    };
    // The case only reports while a bud sits in it.
    let case_note = match (app.case, app.case_stale) {
        (None, _) => Some("dock a bud to read"),
        (Some(_), true) => Some("last seen"),
        _ => None,
    };

    let lines = vec![
        battery_line("Left ", app.left, placement(app.left_in_case)),
        battery_line("Right", app.right, placement(app.right_in_case)),
        battery_line("Case ", app.case, case_note),
    ];
    f.render_widget(Paragraph::new(lines), inner);
}

fn battery_line(label: &str, battery: Option<Battery>, note: Option<&str>) -> Line<'static> {
    let mut spans = vec![Span::styled(format!(" {label}  "), Style::default().fg(FG))];
    match battery {
        Some(b) => {
            let color = match b.level {
                ..=10 => Color::Red,
                11..=25 => Color::Yellow,
                _ => Color::Green,
            };
            let filled = (b.level.clamp(0, 100) as usize + 5) / 10;
            spans.push(Span::styled("█".repeat(filled), Style::default().fg(color)));
            spans.push(Span::styled("░".repeat(10 - filled), Style::default().fg(DIM)));
            spans.push(Span::styled(format!(" {:>3}%", b.level), Style::default().fg(FG)));
            if b.charging {
                spans.push(Span::styled(" ⚡", Style::default().fg(HEADER)));
            }
        }
        None => spans.push(Span::styled("—", Style::default().fg(DIM))),
    }
    if let Some(note) = note {
        spans.push(Span::styled(format!("  {note}"), Style::default().fg(DIM)));
    }
    Line::from(spans)
}

fn draw_settings(f: &mut Frame, area: Rect, app: &App) {
    let block = section("Settings", true);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines = Vec::new();
    for (i, row) in ROWS.iter().enumerate() {
        if *row == Row::Eq(0) {
            lines.push(Line::styled(
                " Equalizer",
                Style::default().fg(HEADER).add_modifier(Modifier::BOLD),
            ));
        }
        lines.push(setting_line(app, *row, i == app.selected));
    }

    // Keep the selection visible on short terminals.
    let height = inner.height as usize;
    let selected_line = app.selected + usize::from(app.selected >= 9);
    let scroll = selected_line.saturating_sub(height.saturating_sub(1));
    f.render_widget(Paragraph::new(lines).scroll((scroll as u16, 0)), inner);
}

fn setting_line(app: &App, row: Row, focused: bool) -> Line<'static> {
    let label = match row {
        Row::Anc => "Noise control",
        Row::Multipoint => "Multipoint",
        Row::OnHead => "On-head detection",
        Row::Speech => "Conversation detection",
        Row::VolumeNotifications => "Volume notifications",
        Row::VolumeEq => "Volume EQ",
        Row::Mono => "Mono audio",
        Row::Gestures => "Touch controls",
        Row::Balance => "Balance",
        Row::Eq(band) => EQ_BANDS[band],
    };

    let (marker, label_style) = if focused {
        ("▶ ", Style::default().fg(FOCUS).add_modifier(Modifier::BOLD))
    } else {
        ("  ", Style::default().fg(FG))
    };

    let mut spans = vec![
        Span::styled(marker, Style::default().fg(FOCUS)),
        Span::styled(format!("{label:<24}"), label_style),
    ];
    spans.extend(match row {
        Row::Anc => anc_value(app.anc),
        Row::Balance => match app.balance {
            Some(b) => slider(b.value() as f32, -100.0, 100.0, balance_text(b.value())),
            None => unknown(),
        },
        Row::Eq(band) => match app.eq {
            Some(eq) => {
                let v = eq_band(&eq, band);
                slider(v, EqBands::MIN_VALUE, EqBands::MAX_VALUE, format!("{v:+.1} dB"))
            }
            None => unknown(),
        },
        row => match app.switch(row) {
            Some(true) => vec![Span::styled("● On", Style::default().fg(FOCUS))],
            Some(false) => vec![Span::styled("○ Off", Style::default().fg(DIM))],
            None => unknown(),
        },
    });
    Line::from(spans)
}

fn anc_value(current: Option<AncState>) -> Vec<Span<'static>> {
    let Some(current) = current else { return unknown() };
    let modes = [
        (AncState::Off, "Off"),
        (AncState::Active, "ANC"),
        (AncState::Aware, "Transparency"),
        (AncState::Adaptive, "Adaptive"),
    ];
    let mut spans = Vec::new();
    for (i, (state, name)) in modes.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw(" "));
        }
        if *state == current {
            spans.push(Span::styled(
                format!("[{name}]"),
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ));
        } else {
            spans.push(Span::styled(name.to_string(), Style::default().fg(DIM)));
        }
    }
    spans
}

fn balance_text(v: i32) -> String {
    match v {
        0 => "centered".into(),
        v if v < 0 => format!("L {}", -v),
        v => format!("R {v}"),
    }
}

/// A centered bar with the marker at `value`, e.g. `──────┼──●───`.
fn slider(value: f32, min: f32, max: f32, text: String) -> Vec<Span<'static>> {
    const W: usize = 21;
    let pos = (((value - min) / (max - min)) * (W - 1) as f32).round() as usize;
    let bar: String = (0..W)
        .map(|i| match i {
            i if i == pos => '●',
            i if i == W / 2 => '┼',
            _ => '─',
        })
        .collect();
    vec![
        Span::styled(bar, Style::default().fg(ACCENT)),
        Span::styled(format!(" {text}"), Style::default().fg(FG)),
    ]
}

fn unknown() -> Vec<Span<'static>> {
    vec![Span::styled("—", Style::default().fg(DIM))]
}

fn draw_footer(f: &mut Frame, area: Rect, app: &App) {
    let line = match &app.message {
        Some((msg, at)) if at.elapsed() < MESSAGE_TTL => {
            Line::styled(msg.clone(), Style::default().fg(Color::Red))
        }
        _ => {
            let key = |k: &'static str| Span::styled(k, Style::default().fg(ACCENT));
            let txt = |t: &'static str| Span::styled(t, Style::default().fg(DIM));
            Line::from(vec![
                key("↑↓"), txt(" move  "),
                key("←→"), txt(" change  "),
                key("⏎"), txt(" toggle  "),
                key("1-4"), txt(" ANC  "),
                key("r"), txt(" reset  "),
                key("i"), txt(" info  "),
                key("q"), txt(" quit"),
            ])
        }
    };
    f.render_widget(Paragraph::new(line).alignment(Alignment::Center), area);
}

fn draw_info(f: &mut Frame, area: Rect, app: &App) {
    let popup = centered_rect(area, 46, 9);
    f.render_widget(Clear, popup);
    let block = section("Device info", true);
    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let row = |k: &str, v: String| {
        Line::from(vec![
            Span::styled(format!(" {k:<15}"), Style::default().fg(DIM)),
            Span::styled(v, Style::default().fg(FG)),
        ])
    };
    let address = app
        .device
        .as_ref()
        .map(|(_, a)| a.to_string())
        .unwrap_or_else(|| "—".into());
    let fw = app.firmware.clone().unwrap_or_else(|| ["—".into(), "—".into(), "—".into()]);
    let [left, right, case] = fw;

    let lines = vec![
        row("Address", address),
        Line::raw(""),
        row("Left firmware", left),
        row("Right firmware", right),
        row("Case firmware", case),
        Line::raw(""),
        Line::styled(" press any key", Style::default().fg(DIM)),
    ];
    f.render_widget(Paragraph::new(lines), inner);
}

fn section(title: &str, accent: bool) -> Block<'_> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(if accent { ACCENT } else { DIM }))
        .title(Span::styled(
            format!(" {title} "),
            Style::default().fg(HEADER).add_modifier(Modifier::BOLD),
        ))
}

fn centered_col(area: Rect, width: u16) -> Rect {
    let w = width.min(area.width);
    Rect { x: area.x + (area.width - w) / 2, width: w, ..area }
}

fn centered_rect(area: Rect, width: u16, height: u16) -> Rect {
    let w = width.min(area.width);
    let h = height.min(area.height);
    Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    }
}
