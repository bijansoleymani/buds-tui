//! The AirPods screen, as far as macOS lets it go.
//!
//! On Linux this screen is built from AACP notifications and is enormous. Here
//! it is noise control plus battery, because that is all macOS exposes of the
//! AACP session it holds — see `docs/macos.md`. The screen is explicit about
//! the difference rather than showing empty rows for settings that can never
//! be filled.

use std::time::{Duration, Instant};

use apmac::{DeviceInfo, Headphones, NoiseMode};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};

const FLASH_FOR: Duration = Duration::from_secs(3);

#[derive(Default)]
pub struct AirPods {
    /// Identity and battery, from `system_profiler`. Polled slowly.
    pub info: Option<DeviceInfo>,
    /// Listening mode, from CoreAudio. Only present while the AirPods are the
    /// active audio output.
    pub pods: Option<Headphones>,
    pub cursor: usize,
    flash: Option<(String, Instant)>,
}

impl AirPods {
    /// The modes to offer. `lsms` is authoritative when the device reports it;
    /// otherwise all four are shown and the write is allowed to fail.
    pub fn modes(&self) -> Vec<NoiseMode> {
        match &self.pods {
            Some(h) if !h.supported.is_empty() => h.supported.clone(),
            _ => NoiseMode::ALL.to_vec(),
        }
    }

    pub fn current(&self) -> Option<NoiseMode> {
        self.pods.as_ref().and_then(|h| h.mode)
    }

    pub fn up(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn down(&mut self) {
        let last = self.modes().len().saturating_sub(1);
        self.cursor = (self.cursor + 1).min(last);
    }

    /// Puts the cursor on whatever mode the buds are actually in, so the
    /// screen does not argue with the device after an external change.
    pub fn sync_cursor(&mut self) {
        if let Some(current) = self.current()
            && let Some(i) = self.modes().iter().position(|m| *m == current)
        {
            self.cursor = i;
        }
    }

    pub fn activate(&mut self) {
        let modes = self.modes();
        if let Some(mode) = modes.get(self.cursor).copied() {
            self.set_mode(mode);
        }
    }

    pub fn set_mode(&mut self, mode: NoiseMode) {
        let Some(pods) = self.pods.as_mut() else {
            self.flash("No listening mode available — make the AirPods the sound output");
            return;
        };
        match pods.set_mode(mode) {
            Ok(()) => {
                let label = mode.label();
                self.flash(format!("{label} requested"));
                self.sync_cursor();
            }
            Err(e) => self.flash(format!("{e}")),
        }
    }

    pub fn flash(&mut self, msg: impl Into<String>) {
        self.flash = Some((msg.into(), Instant::now()));
    }

    fn flash_text(&self) -> Option<&str> {
        self.flash
            .as_ref()
            .filter(|(_, at)| at.elapsed() < FLASH_FOR)
            .map(|(m, _)| m.as_str())
    }
}

/// `██████░░░░ 62%`, coloured by how much is left.
fn battery_line(label: &str, level: Option<u8>) -> Line<'static> {
    let Some(level) = level else {
        return Line::from(vec![
            Span::styled(
                format!("  {label:<8}"),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled("—", Style::default().fg(Color::DarkGray)),
        ]);
    };
    let colour = match level {
        0..=10 => Color::Red,
        11..=20 => Color::LightRed,
        21..=40 => Color::Yellow,
        _ => Color::Green,
    };
    let filled = (level as usize * 10).div_ceil(100);
    let bar: String = "█".repeat(filled) + &"░".repeat(10 - filled);
    Line::from(vec![
        Span::raw(format!("  {label:<8}")),
        Span::styled(bar, Style::default().fg(colour)),
        Span::styled(format!(" {level}%"), Style::default().fg(colour)),
    ])
}

pub fn draw_in(f: &mut Frame, area: Rect, ap: &AirPods) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(match &ap.info {
            Some(d) => format!(" {} ", d.name),
            None => " AirPods ".to_string(),
        });
    let inner = block.inner(area);
    f.render_widget(block, area);

    let [top, modes, footer] = Layout::vertical([
        Constraint::Length(7),
        Constraint::Min(6),
        Constraint::Length(3),
    ])
    .areas(inner);

    // ── identity and battery ──
    let mut lines = Vec::new();
    match &ap.info {
        Some(d) => {
            lines.push(Line::from(vec![
                Span::styled("  Address  ", Style::default().fg(Color::DarkGray)),
                Span::raw(d.address.clone()),
            ]));
            if let Some(fw) = &d.firmware {
                lines.push(Line::from(vec![
                    Span::styled("  Firmware ", Style::default().fg(Color::DarkGray)),
                    Span::raw(fw.clone()),
                ]));
            }
            lines.push(Line::from(""));
            lines.push(battery_line("Left", d.battery_left.or(d.battery)));
            lines.push(battery_line("Right", d.battery_right));
            if d.battery_case.is_some() {
                lines.push(battery_line("Case", d.battery_case));
            }
        }
        None => {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "  No AirPods connected to this Mac.",
                Style::default().fg(Color::DarkGray),
            )));
        }
    }
    f.render_widget(Paragraph::new(lines), top);

    // ── noise control ──
    let mut rows = vec![Line::from(Span::styled(
        " Noise control",
        Style::default().add_modifier(Modifier::BOLD),
    ))];
    if ap.pods.is_some() {
        let current = ap.current();
        for (i, mode) in ap.modes().iter().enumerate() {
            let selected = i == ap.cursor;
            let active = Some(*mode) == current;
            let marker = if active { "●" } else { "○" };
            let style = if active {
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD)
            } else if selected {
                Style::default().fg(Color::White)
            } else {
                Style::default().fg(Color::Gray)
            };
            let label = if selected {
                format!("  {marker} [{}]", mode.label())
            } else {
                format!("  {marker} {}", mode.label())
            };
            rows.push(Line::from(vec![
                Span::styled(format!(" {}", i + 1), Style::default().fg(Color::DarkGray)),
                Span::styled(label, style),
            ]));
        }
    } else {
        rows.push(Line::from(""));
        rows.push(Line::from(Span::styled(
            "  Unavailable — macOS only exposes the listening mode",
            Style::default().fg(Color::DarkGray),
        )));
        rows.push(Line::from(Span::styled(
            "  while the AirPods are the active sound output.",
            Style::default().fg(Color::DarkGray),
        )));
    }
    f.render_widget(Paragraph::new(rows), modes);

    // ── footer ──
    let footer_line = match ap.flash_text() {
        Some(msg) => Line::from(Span::styled(
            msg.to_string(),
            Style::default().fg(Color::Yellow),
        )),
        None => Line::from(Span::styled(
            "Conversation awareness, ear detection and the other AACP settings \
             are not reachable on macOS.",
            Style::default().fg(Color::DarkGray),
        )),
    };
    f.render_widget(
        Paragraph::new(footer_line).alignment(Alignment::Center),
        footer,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    /// Renders the screen and returns it as lines of text, so assertions can
    /// be about what a user would actually see.
    fn render(ap: &AirPods) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(78, 22)).unwrap();
        terminal.draw(|f| draw_in(f, f.area(), ap)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    fn pods_with(mode: NoiseMode) -> AirPods {
        AirPods {
            info: Some(DeviceInfo {
                name: "AirPods Pro #2".into(),
                address: "98:1C:A2:BF:ED:74".into(),
                connected: true,
                vendor_id: Some(0x004C),
                firmware: Some("9A348".into()),
                battery_left: Some(99),
                battery_right: Some(100),
                ..Default::default()
            }),
            pods: Some(Headphones::synthetic(
                "AirPods Pro #2",
                Some(mode),
                vec![
                    NoiseMode::Off,
                    NoiseMode::NoiseCancellation,
                    NoiseMode::Transparency,
                ],
            )),
            ..Default::default()
        }
    }

    #[test]
    fn cursor_stays_inside_the_mode_list() {
        let mut ap = AirPods::default();
        ap.up();
        assert_eq!(ap.cursor, 0);
        for _ in 0..20 {
            ap.down();
        }
        assert_eq!(ap.cursor, NoiseMode::ALL.len() - 1);
    }

    #[test]
    fn offers_all_modes_when_the_device_reports_none() {
        let ap = AirPods::default();
        assert_eq!(ap.modes(), NoiseMode::ALL.to_vec());
    }

    #[test]
    fn setting_a_mode_without_a_device_explains_itself() {
        let mut ap = AirPods::default();
        ap.set_mode(NoiseMode::Transparency);
        assert!(ap.flash_text().unwrap().contains("sound output"));
    }

    #[test]
    fn battery_bar_fills_proportionally() {
        for (level, expected) in [(100u8, 10usize), (50, 5), (5, 1), (0, 0)] {
            let line = battery_line("Left", Some(level));
            let bar = line.spans[1].content.as_ref();
            assert_eq!(
                bar.chars().filter(|c| *c == '█').count(),
                expected,
                "{level}%"
            );
            assert_eq!(bar.chars().count(), 10);
        }
    }

    #[test]
    fn renders_battery_and_the_active_mode() {
        let mut ap = pods_with(NoiseMode::Transparency);
        ap.sync_cursor();
        let screen = render(&ap).join("\n");

        assert!(screen.contains("AirPods Pro #2"), "{screen}");
        assert!(screen.contains("98:1C:A2:BF:ED:74"), "{screen}");
        assert!(screen.contains("9A348"), "{screen}");
        // Both buds, with their own levels.
        assert!(screen.contains("99%"), "{screen}");
        assert!(screen.contains("100%"), "{screen}");
        // Transparency is active, so it is the filled marker and the cursor.
        assert!(screen.contains("● [Transparency]"), "{screen}");
        // Adaptive is not in the reported support mask, so it is not offered.
        assert!(!screen.contains("Adaptive"), "{screen}");
    }

    #[test]
    fn explains_itself_when_macos_exposes_no_mode() {
        let mut ap = pods_with(NoiseMode::Off);
        ap.pods = None;
        let screen = render(&ap).join("\n");
        assert!(screen.contains("Unavailable"), "{screen}");
        assert!(screen.contains("active sound output"), "{screen}");
        // Battery still comes from system_profiler, so it must still show.
        assert!(screen.contains("99%"), "{screen}");
    }

    #[test]
    fn renders_without_any_airpods() {
        let screen = render(&AirPods::default()).join("\n");
        assert!(screen.contains("No AirPods connected"), "{screen}");
    }

    #[test]
    fn missing_battery_renders_a_dash() {
        let line = battery_line("Case", None);
        assert_eq!(line.spans[1].content.as_ref(), "—");
    }
}
