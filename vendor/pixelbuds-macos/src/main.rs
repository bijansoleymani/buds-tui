//! Terminal UI for Google Pixel Buds Pro / Pro 2 on macOS.
//!
//! Talks to the buds over Google's Maestro protocol (the one the Pixel Buds
//! app uses), via the `maestro` crate from qzed/pbpctrl, carried over an
//! IOBluetooth RFCOMM channel.
//!
//! # Why the threads are arranged this way
//!
//! IOBluetooth only opens a channel from the process's main thread, and only
//! delivers its callbacks to a run loop running there — a thread of our own
//! with its own run loop is refused with `kIOReturnError`. So main belongs to
//! the run loop, and the terminal UI and the Maestro session run on a thread
//! of their own. The Linux build has main the other way round.

use std::time::Duration;

use anyhow::Result;
use btmac::Address;
use clap::Parser;
use crossterm::event::{Event, EventStream};
use futures::StreamExt;
use tokio::sync::mpsc;

use pixelbuds_macos::app::App;
use pixelbuds_macos::{handle_key, link, ui};

#[derive(Parser)]
#[command(version, about)]
struct Args {
    /// Bluetooth address of the buds. Without it, the first connected device
    /// offering the Maestro service is used.
    #[arg(short, long)]
    device: Option<Address>,

    /// Skip the UI: print what the buds report for this many seconds, then
    /// exit. Useful without a terminal, and for checking the link by itself.
    #[arg(long, value_name = "SECONDS")]
    probe: Option<u64>,
}

fn main() -> Result<()> {
    let args = Args::parse();
    btmac::init();

    std::thread::Builder::new()
        .name("pixelbuds-ui".into())
        .stack_size(2 * 1024 * 1024)
        .spawn(move || {
            let code = match args.probe {
                Some(seconds) => run_probe(args.device, seconds),
                None => run_ui(args.device),
            };
            let code = match code {
                Ok(()) => 0,
                Err(e) => {
                    eprintln!("pixelbuds-macos: {e:#}");
                    1
                }
            };
            // main is parked in the run loop and will never return, so the UI
            // thread is the one that ends the process.
            std::process::exit(code);
        })?;

    btmac::run_main_loop();
}

/// Drives the link with no UI and prints every event, so the Bluetooth half
/// can be checked on its own and without a terminal.
fn run_probe(device: Option<Address>, seconds: u64) -> Result<()> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()?;
    rt.block_on(async move {
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        let (_cmd_tx, cmd_rx) = mpsc::unbounded_channel();

        let link = link::run(device, event_tx, cmd_rx);
        tokio::pin!(link);

        let deadline = tokio::time::sleep(Duration::from_secs(seconds));
        tokio::pin!(deadline);

        let mut events = 0usize;
        loop {
            tokio::select! {
                _ = &mut link => {}
                _ = &mut deadline => break,
                Some(ev) = event_rx.recv() => {
                    events += 1;
                    println!("{ev:?}");
                }
            }
        }
        println!("-- {events} event(s) in {seconds}s");
        if events == 0 {
            anyhow::bail!("the link produced nothing; are the buds connected to this Mac?");
        }
        Ok(())
    })
}

/// The original single-threaded UI loop, now on a thread of its own.
fn run_ui(device: Option<Address>) -> Result<()> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()?;
    rt.block_on(async move {
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let mut app = App::new(cmd_tx);

        // MaestroService is not Send, so the link runs on this thread rather
        // than in a spawned task.
        let link = link::run(device, event_tx, cmd_rx);
        tokio::pin!(link);

        let mut terminal = ratatui::init();
        let mut keys = EventStream::new();
        // Redraws expire flash messages.
        let mut tick = tokio::time::interval(Duration::from_secs(1));

        let result: Result<()> = async {
            while !app.quit {
                terminal.draw(|f| ui::draw(f, &app))?;
                tokio::select! {
                    _ = &mut link => {}
                    Some(ev) = event_rx.recv() => {
                        app.apply(ev);
                        while let Ok(ev) = event_rx.try_recv() {
                            app.apply(ev);
                        }
                    }
                    Some(ev) = keys.next() => {
                        if let Event::Key(key) = ev? {
                            handle_key(&mut app, key);
                        }
                    }
                    _ = tick.tick() => {}
                }
            }
            Ok(())
        }
        .await;

        ratatui::restore();
        result
    })
}
