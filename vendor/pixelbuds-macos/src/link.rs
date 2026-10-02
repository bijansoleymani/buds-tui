//! Owns the Bluetooth side: finds the Pixel Buds, opens the Maestro RFCOMM
//! channel, and turns what the buds report into `LinkEvent`s for the UI.
//! Setting writes come back the other way as `SettingValue`s.
//!
//! The Maestro session resets whenever the buds hand processing off between
//! each other (pbpctrl documents this as `os error 104`), so every failure
//! just waits briefly and reconnects.
//!
//! This is the macOS port of the original BlueZ version. Everything above the
//! byte stream is untouched — the `maestro` client is pure protocol — but the
//! transport is now `btmac`, over IOBluetooth. Two differences are worth
//! knowing:
//!
//!   * BlueZ needed a profile registered and an inbound connection request
//!     accepted to get a socket. IOBluetooth just opens the channel against
//!     the RFCOMM channel id from the device's SDP records, so that whole
//!     dance is gone.
//!   * Opening a channel is a blocking call that has to run on the process's
//!     main thread, so it goes through `spawn_blocking` rather than being
//!     awaited directly, which also keeps the UI drawing while it happens.
//!
//! Parts of this file are adapted from pbpctrl (cli/src/bt.rs) and
//! omarchy-pixelbuds (daemon/pixelbudsd/src/maestro_link.rs), both MIT;
//! see NOTICE.

use std::time::Duration;

use anyhow::{Context, Result};
use btmac::{Address, Channel, Device};
use futures::StreamExt;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use maestro::protocol::codec::Codec;
use maestro::protocol::types::{RuntimeInfo, SoftwareInfo, settings_rsp};
use maestro::protocol::utils;
use maestro::pwrpc::client::Client;
use maestro::service::MaestroService;
use maestro::service::settings::{SettingId, SettingValue};

#[derive(Debug)]
pub enum LinkEvent {
    /// Free-form state for the header while there is no session.
    Status(String),
    Device { name: String, address: Address },
    /// No suitable device is connected over Bluetooth.
    Absent,
    Connected,
    Disconnected,
    Runtime(RuntimeInfo),
    Setting(SettingValue),
    Firmware(SoftwareInfo),
    Error(String),
}

const SETTINGS_TO_SEED: [SettingId; 10] = [
    SettingId::CurrentAncrState,
    SettingId::MultipointEnable,
    SettingId::OhdEnable,
    SettingId::SpeechDetection,
    SettingId::VolumeExposureNotifications,
    SettingId::VolumeEqEnable,
    SettingId::SumToMono,
    SettingId::GestureEnable,
    SettingId::VolumeAsymmetry,
    SettingId::CurrentUserEq,
];

pub async fn run(
    wanted: Option<Address>,
    events: UnboundedSender<LinkEvent>,
    mut commands: UnboundedReceiver<SettingValue>,
) {
    loop {
        let dev = match find_device(wanted) {
            Ok(Some(dev)) => dev,
            Ok(None) => {
                let _ = events.send(LinkEvent::Absent);
                let _ = events.send(LinkEvent::Status(
                    "Waiting for Pixel Buds to connect…".into(),
                ));
                drain(&mut commands);
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            }
            Err(e) => {
                let _ = events.send(LinkEvent::Absent);
                let _ = events.send(LinkEvent::Status(format!("Bluetooth error: {e}")));
                tokio::time::sleep(Duration::from_secs(3)).await;
                continue;
            }
        };

        let name = if dev.name.is_empty() {
            dev.addr.to_string()
        } else {
            dev.name.clone()
        };
        let _ = events.send(LinkEvent::Device { name, address: dev.addr });
        let _ = events.send(LinkEvent::Status("Connecting…".into()));

        if let Err(e) = run_session(&dev, &events, &mut commands).await {
            let _ = events.send(LinkEvent::Status(format!("Reconnecting ({e:#})")));
        }
        let _ = events.send(LinkEvent::Disconnected);
        drain(&mut commands);
        tokio::time::sleep(Duration::from_millis(1500)).await;
    }
}

/// Writes queued while disconnected would land on whatever state the buds
/// have by the time we reconnect, so drop them instead.
fn drain(commands: &mut UnboundedReceiver<SettingValue>) {
    while commands.try_recv().is_ok() {}
}

/// The requested device if it is connected, otherwise the first connected
/// device that offers the Maestro service. Only connected devices count:
/// opening the TUI should not pull the buds away from a phone.
fn find_device(wanted: Option<Address>) -> Result<Option<Device>> {
    btmac::find_maestro_device(wanted).map_err(Into::into)
}

async fn run_session(
    dev: &Device,
    events: &UnboundedSender<LinkEvent>,
    commands: &mut UnboundedReceiver<SettingValue>,
) -> Result<()> {
    let stream = connect_maestro_rfcomm(dev.addr).await?;

    let mut client = Client::new(Codec::new().wrap(stream));
    let handle = client.handle();
    let channel = utils::resolve_channel(&mut client)
        .await
        .context("resolving Maestro channel")?;
    let service = MaestroService::new(handle, channel);

    // `client.run()` is what moves requests and replies over the socket, so
    // it has to be polled alongside every RPC below.
    let client_task = async move {
        client.run().await?;
        anyhow::bail!("connection closed")
    };

    tokio::select! {
        res = client_task => res,
        res = serve(service, events, commands) => res,
    }
}

async fn serve(
    mut service: MaestroService,
    events: &UnboundedSender<LinkEvent>,
    commands: &mut UnboundedReceiver<SettingValue>,
) -> Result<()> {
    let _ = events.send(LinkEvent::Connected);

    // Subscriptions only report changes, so read every setting once first.
    for id in SETTINGS_TO_SEED {
        if let Ok(value) = service.read_setting_var(id).await {
            let _ = events.send(LinkEvent::Setting(value));
        }
    }
    if let Ok(info) = service.get_software_info().await {
        let _ = events.send(LinkEvent::Firmware(info));
    }

    let runtime = {
        let mut service = service.clone();
        async move {
            let mut call = service.subscribe_to_runtime_info()?;
            while let Some(msg) = call.stream().next().await {
                let _ = events.send(LinkEvent::Runtime(msg?));
            }
            anyhow::bail!("runtime info stream ended")
        }
    };

    let settings = {
        let mut service = service.clone();
        async move {
            let mut call = service.subscribe_to_settings_changes()?;
            while let Some(msg) = call.stream().next().await {
                let Some(settings_rsp::ValueOneof::Value(raw)) = msg?.value_oneof else {
                    continue;
                };
                if let Some(value) = raw.value_oneof {
                    let _ = events.send(LinkEvent::Setting(value.into()));
                }
            }
            anyhow::bail!("settings stream ended")
        }
    };

    let writes = async move {
        while let Some(value) = commands.recv().await {
            if let Err(e) = service.write_setting(value).await {
                let _ = events.send(LinkEvent::Error(format!("write failed: {e}")));
            }
        }
        Ok(())
    };

    tokio::select! {
        res = runtime => res,
        res = settings => res,
        res = writes => res,
    }
}

/// Opens the Maestro channel. IOBluetooth insists the SDP lookup and the
/// channel open both happen on the process's main thread, and blocks while it
/// connects, so this hands the work to the blocking pool; the caller's thread
/// stays free to keep drawing.
async fn connect_maestro_rfcomm(addr: Address) -> Result<Channel> {
    tokio::task::spawn_blocking(move || {
        let channel_id = btmac::rfcomm_channel_for_uuid(addr, &btmac::MAESTRO_UUID)
            .context("looking up the Maestro RFCOMM channel in SDP")?;
        btmac::open_rfcomm(addr, channel_id).context("opening the Maestro RFCOMM channel")
    })
    .await
    .context("the Bluetooth open task panicked")?
}
