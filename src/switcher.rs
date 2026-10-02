//! Picks which earbuds screen to show, AirPods or Pixel Buds, from what is
//! connected over Bluetooth, and draws the tab line shown when both are.

use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::tui::app::{App, DeviceState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    AirPods,
    PixelBuds,
}

/// Follows connections: a pair that has just connected comes to the front,
/// one that is the only pair connected stays there, and with nothing
/// connected the AirPods screen shows (it also lists AirPods merely nearby).
pub struct Switcher {
    pub screen: Screen,
    airpods_was: bool,
    pixel_was: bool,
}

impl Switcher {
    pub fn new() -> Self {
        Self { screen: Screen::AirPods, airpods_was: false, pixel_was: false }
    }

    pub fn update(&mut self, airpods: bool, pixel: bool) {
        if airpods && !self.airpods_was {
            self.screen = Screen::AirPods;
        }
        if pixel && !self.pixel_was {
            self.screen = Screen::PixelBuds;
        }
        match (airpods, pixel) {
            (true, false) | (false, false) => self.screen = Screen::AirPods,
            (false, true) => self.screen = Screen::PixelBuds,
            (true, true) => {}
        }
        self.airpods_was = airpods;
        self.pixel_was = pixel;
    }

    /// Manual switch, only meaningful while both pairs are connected.
    pub fn toggle(&mut self) {
        if self.airpods_was && self.pixel_was {
            self.screen = match self.screen {
                Screen::AirPods => Screen::PixelBuds,
                Screen::PixelBuds => Screen::AirPods,
            };
        }
    }

    pub fn both(&self) -> bool {
        self.airpods_was && self.pixel_was
    }
}

pub fn airpods_connected(app: &App) -> bool {
    app.devices
        .values()
        .any(|d| matches!(d, DeviceState::AirPods(s) if s.connected))
}

/// Splits off a tab line when both pairs are connected and returns the area
/// left for the active screen.
pub fn draw_tabs(f: &mut Frame, switcher: &Switcher) -> Rect {
    let area = f.area();
    if !switcher.both() {
        return area;
    }
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Fill(1)])
        .split(area);

    let tab = |label: &'static str, active: bool| {
        if active {
            Span::styled(
                format!(" {label} "),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            )
        } else {
            Span::styled(format!(" {label} "), Style::default().fg(Color::DarkGray))
        }
    };
    let line = Line::from(vec![
        tab("AirPods", switcher.screen == Screen::AirPods),
        Span::raw("  "),
        tab("Pixel Buds", switcher.screen == Screen::PixelBuds),
        Span::styled("   b", Style::default().fg(Color::Cyan)),
        Span::styled(" switch", Style::default().fg(Color::DarkGray)),
    ]);
    f.render_widget(Paragraph::new(line).alignment(Alignment::Center), chunks[0]);
    chunks[1]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follows_the_only_connected_pair() {
        let mut s = Switcher::new();
        s.update(false, true);
        assert_eq!(s.screen, Screen::PixelBuds);
        s.update(true, false);
        assert_eq!(s.screen, Screen::AirPods);
        s.update(false, false);
        assert_eq!(s.screen, Screen::AirPods);
    }

    #[test]
    fn newest_connection_wins_and_b_toggles() {
        let mut s = Switcher::new();
        s.update(true, false);
        s.update(true, true);
        assert_eq!(s.screen, Screen::PixelBuds);
        s.toggle();
        assert_eq!(s.screen, Screen::AirPods);
        // Staying connected does not undo the manual choice.
        s.update(true, true);
        assert_eq!(s.screen, Screen::AirPods);
        // Pixel Buds leave: back to the one still connected.
        s.update(true, false);
        assert_eq!(s.screen, Screen::AirPods);
    }

    #[test]
    fn toggle_is_ignored_with_one_pair() {
        let mut s = Switcher::new();
        s.update(true, false);
        s.toggle();
        assert_eq!(s.screen, Screen::AirPods);
    }
}
