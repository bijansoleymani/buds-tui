//! AirPods and Pixel Buds in one terminal UI, on macOS.
//!
//! The macOS counterpart of the `buds-tui` binary. Same idea — one screen per
//! kind of bud, `b` to switch, a tab line when both are around — but the two
//! halves get their data very differently here:
//!
//!   * Pixel Buds: the real Maestro protocol over an IOBluetooth RFCOMM
//!     channel, which is the Linux behaviour unchanged (`btmac`).
//!   * AirPods: not AACP, which macOS keeps to itself, but what macOS exposes
//!     of the session it holds — CoreAudio for noise control, system_profiler
//!     for battery (`apmac`).
//!
//! `main` belongs to the IOBluetooth run loop, so the UI runs on a thread of
//! its own; see the `btmac` crate docs for why that is not optional.

mod airpods;

use std::time::Duration;

use anyhow::Result;
use btmac::Address;
use clap::Parser;
use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use futures::StreamExt;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use tokio::sync::mpsc;

use apmac::NoiseMode;
use pixelbuds_macos::app::App as BudsApp;
use pixelbuds_macos::link::LinkEvent;

/// How often to re-read the listening mode. Cheap: a CoreAudio property read.
const MODE_POLL: Duration = Duration::from_secs(1);
/// How often to re-read battery. Expensive: spawns `system_profiler`.
const INFO_POLL: Duration = Duration::from_secs(5);

#[derive(Parser)]
#[command(version, about)]
struct Args {
    /// Bluetooth address of the Pixel Buds. Without it, the first connected
    /// device offering the Maestro service is used.
    #[arg(short, long)]
    device: Option<Address>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    AirPods,
    PixelBuds,
}

fn main() -> Result<()> {
    let args = Args::parse();
    btmac::init();

    std::thread::Builder::new()
        .name("buds-ui".into())
        .stack_size(2 * 1024 * 1024)
        .spawn(move || {
            let code = match run_ui(args.device) {
                Ok(()) => 0,
                Err(e) => {
                    eprintln!("buds-macos: {e:#}");
                    1
                }
            };
            // main never returns from the run loop, so the UI thread ends the
            // process.
            std::process::exit(code);
        })?;

    btmac::run_main_loop();
}

fn run_ui(device: Option<Address>) -> Result<()> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()?;
    rt.block_on(async move {
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let mut buds = BudsApp::new(cmd_tx);
        let mut pods = airpods::AirPods::default();

        // The Maestro service is not Send, so the link shares this thread.
        let link = pixelbuds_macos::link::run(device, event_tx, cmd_rx);
        tokio::pin!(link);

        // system_profiler is a subprocess, so it goes to the blocking pool and
        // reports back rather than being awaited in the draw loop.
        let (info_tx, mut info_rx) = mpsc::unbounded_channel();

        let mut screen = Screen::AirPods;
        // Whether the Pixel Buds screen has already been brought forward for
        // the current connection; see `note_link_event`.
        let mut buds_fronted = false;
        let mut quit = false;

        let mut terminal = ratatui::init();
        let mut keys = EventStream::new();
        let mut mode_tick = tokio::time::interval(MODE_POLL);
        let mut info_tick = tokio::time::interval(INFO_POLL);

        let result: Result<()> = async {
            while !quit && !buds.quit {
                terminal.draw(|f| draw(f, screen, &pods, &buds))?;
                tokio::select! {
                    _ = &mut link => {}
                    Some(ev) = event_rx.recv() => {
                        note_link_event(&ev, &mut screen, &mut buds_fronted);
                        buds.apply(ev);
                        while let Ok(ev) = event_rx.try_recv() {
                            note_link_event(&ev, &mut screen, &mut buds_fronted);
                            buds.apply(ev);
                        }
                    }
                    Some(info) = info_rx.recv() => {
                        pods.info = info;
                    }
                    Some(ev) = keys.next() => {
                        if let Event::Key(key) = ev? {
                            handle_key(key, &mut screen, &mut pods, &mut buds, &mut quit);
                        }
                    }
                    _ = mode_tick.tick() => {
                        pods.pods = apmac::mode::active();
                        // Follow the device, so a change made elsewhere (Control
                        // Centre, the stem) moves the cursor rather than being
                        // argued with. A no-op when there is no mode to read.
                        pods.sync_cursor();
                    }
                    _ = info_tick.tick() => {
                        let tx = info_tx.clone();
                        tokio::task::spawn_blocking(move || {
                            let first = apmac::battery::connected_airpods()
                                .ok()
                                .and_then(|v| v.into_iter().next());
                            let _ = tx.send(first);
                        });
                    }
                }
            }
            Ok(())
        }
        .await;

        ratatui::restore();
        result
    })
}

/// Brings the Pixel Buds screen forward when the buds connect, the way the
/// Linux build brings whichever pair connected last to the front.
///
/// Only once per connection. The Maestro session resets whenever the buds hand
/// processing between each other, which produces a `Disconnected` followed by
/// a fresh `Connected`; switching on those would drag the user off the AirPods
/// screen repeatedly. `Absent` is the only event that means the buds really
/// went away, so it is what arms this again.
fn note_link_event(ev: &LinkEvent, screen: &mut Screen, fronted: &mut bool) {
    match ev {
        LinkEvent::Absent => *fronted = false,
        LinkEvent::Connected if !*fronted => {
            *fronted = true;
            *screen = Screen::PixelBuds;
        }
        _ => {}
    }
}

fn handle_key(
    key: KeyEvent,
    screen: &mut Screen,
    pods: &mut airpods::AirPods,
    buds: &mut BudsApp,
    quit: &mut bool,
) {
    if key.kind != KeyEventKind::Press {
        return;
    }
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        *quit = true;
        return;
    }
    // `b` switches screens from either side, as on Linux.
    if key.code == KeyCode::Char('b') {
        *screen = match screen {
            Screen::AirPods => Screen::PixelBuds,
            Screen::PixelBuds => Screen::AirPods,
        };
        return;
    }
    match screen {
        // The Pixel Buds screen brings its own key handling.
        Screen::PixelBuds => pixelbuds_macos::handle_key(buds, key),
        Screen::AirPods => match key.code {
            KeyCode::Char('q') | KeyCode::Esc => *quit = true,
            KeyCode::Up | KeyCode::Char('k') => pods.up(),
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => pods.down(),
            KeyCode::Enter | KeyCode::Char(' ') => pods.activate(),
            KeyCode::Char('1') => pods.set_mode(NoiseMode::Off),
            KeyCode::Char('2') => pods.set_mode(NoiseMode::NoiseCancellation),
            KeyCode::Char('3') => pods.set_mode(NoiseMode::Transparency),
            KeyCode::Char('4') => pods.set_mode(NoiseMode::Adaptive),
            _ => {}
        },
    }
}

fn draw(f: &mut Frame, screen: Screen, pods: &airpods::AirPods, buds: &BudsApp) {
    let [tabs, body, hints] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(10),
        Constraint::Length(1),
    ])
    .areas(f.area());

    draw_tabs(f, tabs, screen, pods, buds);
    match screen {
        Screen::AirPods => airpods::draw_in(f, body, pods),
        Screen::PixelBuds => pixelbuds_macos::ui::draw_in(f, body, buds),
    }
    draw_hints(f, hints, screen);
}

fn draw_tabs(
    f: &mut Frame,
    area: ratatui::layout::Rect,
    screen: Screen,
    pods: &airpods::AirPods,
    buds: &BudsApp,
) {
    let selected = Style::default()
        .fg(Color::Black)
        .bg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let idle = Style::default().fg(Color::Gray);
    let absent = Style::default().fg(Color::DarkGray);

    let airpods_present = pods.info.is_some();
    let buds_present = buds.device.is_some();

    let tab = |label: &str, active: bool, present: bool| -> Span<'static> {
        let text = format!(" {label} ");
        Span::styled(
            text,
            if active {
                selected
            } else if present {
                idle
            } else {
                absent
            },
        )
    };

    f.render_widget(
        Paragraph::new(Line::from(vec![
            tab("AirPods", screen == Screen::AirPods, airpods_present),
            Span::raw(" "),
            tab("Pixel Buds", screen == Screen::PixelBuds, buds_present),
        ])),
        area,
    );
}

fn draw_hints(f: &mut Frame, area: ratatui::layout::Rect, screen: Screen) {
    let common = "b switch  q quit";
    let text = match screen {
        Screen::AirPods => format!("↑↓ move  ⏎ apply  1-4 mode  {common}"),
        Screen::PixelBuds => {
            format!("↑↓ move  ←→ change  ⏎ toggle  1-4 ANC  r reset  i info  {common}")
        }
    };
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            text,
            Style::default().fg(Color::DarkGray),
        ))),
        area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drives a sequence of events and reports the screen it ends on.
    fn after(events: &[LinkEvent], start: Screen) -> Screen {
        let mut screen = start;
        let mut fronted = false;
        for ev in events {
            note_link_event(ev, &mut screen, &mut fronted);
        }
        screen
    }

    #[test]
    fn connecting_the_buds_brings_their_screen_forward() {
        assert!(matches!(
            after(&[LinkEvent::Connected], Screen::AirPods),
            Screen::PixelBuds
        ));
    }

    #[test]
    fn a_session_reset_does_not_steal_the_screen_back() {
        // Connected, user switches to AirPods, then the Maestro session bounces.
        let mut screen = Screen::AirPods;
        let mut fronted = false;
        note_link_event(&LinkEvent::Connected, &mut screen, &mut fronted);
        assert!(matches!(screen, Screen::PixelBuds));

        screen = Screen::AirPods; // the user presses `b`
        for ev in [LinkEvent::Disconnected, LinkEvent::Connected] {
            note_link_event(&ev, &mut screen, &mut fronted);
        }
        assert!(
            matches!(screen, Screen::AirPods),
            "a reconnect within one session must not pull focus"
        );
    }

    #[test]
    fn a_genuine_reconnect_brings_it_forward_again() {
        let mut screen = Screen::AirPods;
        let mut fronted = false;
        note_link_event(&LinkEvent::Connected, &mut screen, &mut fronted);
        screen = Screen::AirPods; // the user presses `b`

        // The buds actually leave, then come back.
        for ev in [
            LinkEvent::Disconnected,
            LinkEvent::Absent,
            LinkEvent::Connected,
        ] {
            note_link_event(&ev, &mut screen, &mut fronted);
        }
        assert!(matches!(screen, Screen::PixelBuds));
    }

    #[test]
    fn other_events_leave_the_screen_alone() {
        assert!(matches!(
            after(
                &[
                    LinkEvent::Status("Connecting…".into()),
                    LinkEvent::Disconnected,
                    LinkEvent::Error("nope".into()),
                ],
                Screen::AirPods
            ),
            Screen::AirPods
        ));
    }
}
