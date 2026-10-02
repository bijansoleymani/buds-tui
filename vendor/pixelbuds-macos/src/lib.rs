//! Pixel Buds Pro / Pro 2 frontend: the Maestro link, the UI state and its
//! rendering. Used by the `pixelbuds-tui` binary and by programs that embed
//! this screen next to others.

pub mod app;
pub mod link;
pub mod ui;

use btmac::Address;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use maestro::service::settings::AncState;
use tokio::sync::mpsc::{self, UnboundedReceiver};

use app::App;
use link::LinkEvent;

/// Runs the Bluetooth link on a thread of its own, for hosts whose UI loop is
/// not a single-threaded tokio runtime (MaestroService is not Send, so the
/// link cannot be spawned onto a multi-threaded one).
pub fn spawn_link(device: Option<Address>) -> std::io::Result<(App, UnboundedReceiver<LinkEvent>)> {
    let (event_tx, event_rx) = mpsc::unbounded_channel();
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    std::thread::Builder::new()
        .name("pixelbuds-link".into())
        .spawn(move || rt.block_on(link::run(device, event_tx, cmd_rx)))?;
    Ok((App::new(cmd_tx), event_rx))
}

pub fn handle_key(app: &mut App, key: KeyEvent) {
    if key.kind != KeyEventKind::Press {
        return;
    }
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        app.quit = true;
        return;
    }
    if app.show_info {
        app.show_info = false;
        return;
    }
    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => app.quit = true,
        KeyCode::Up | KeyCode::Char('k') => app.up(),
        KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => app.down(),
        KeyCode::Left | KeyCode::Char('h') => app.adjust(false),
        KeyCode::Right | KeyCode::Char('l') => app.adjust(true),
        KeyCode::Enter | KeyCode::Char(' ') => app.activate(),
        KeyCode::Char('r') => app.reset(),
        KeyCode::Char('i') => app.show_info = true,
        KeyCode::Char('1') => app.set_anc(AncState::Off),
        KeyCode::Char('2') => app.set_anc(AncState::Active),
        KeyCode::Char('3') => app.set_anc(AncState::Aware),
        KeyCode::Char('4') => app.set_anc(AncState::Adaptive),
        _ => {}
    }
}
