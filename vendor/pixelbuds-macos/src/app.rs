//! UI state and the mapping from key presses to setting writes.

use std::time::Instant;

use btmac::Address;
use maestro::protocol::types::{DeviceBatteryInfo, FirmwareVersion, RuntimeInfo};
use maestro::service::settings::{
    AncState, EqBands, SettingValue, VolumeAsymmetry,
};
use tokio::sync::mpsc::UnboundedSender;

use crate::link::LinkEvent;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Battery {
    pub level: i32,
    pub charging: bool,
}

impl Battery {
    fn from_wire(info: Option<DeviceBatteryInfo>) -> Option<Self> {
        // BatteryState: 1 = not charging, 2 = charging.
        info.map(|b| Battery { level: b.level, charging: b.state == 2 })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    Anc,
    Multipoint,
    OnHead,
    Speech,
    VolumeNotifications,
    VolumeEq,
    Mono,
    Gestures,
    Balance,
    Eq(usize),
}

pub const ROWS: [Row; 14] = [
    Row::Anc,
    Row::Multipoint,
    Row::OnHead,
    Row::Speech,
    Row::VolumeNotifications,
    Row::VolumeEq,
    Row::Mono,
    Row::Gestures,
    Row::Balance,
    Row::Eq(0),
    Row::Eq(1),
    Row::Eq(2),
    Row::Eq(3),
    Row::Eq(4),
];

pub const EQ_BANDS: [&str; 5] = ["Low bass", "Bass", "Mid", "Treble", "Upper treble"];
const EQ_STEP: f32 = 0.5;
const BALANCE_STEP: i32 = 5;

/// The cycle order the buds themselves use for the ANC gesture.
const ANC_ORDER: [AncState; 4] = [
    AncState::Active,
    AncState::Off,
    AncState::Aware,
    AncState::Adaptive,
];

pub struct App {
    pub status: String,
    pub device: Option<(String, Address)>,
    /// Connected over Bluetooth, whether or not the Maestro session is up
    /// (it drops briefly whenever the buds hand off between each other).
    pub present: bool,
    /// The Maestro session is up and settings can be written.
    pub connected: bool,

    pub left: Option<Battery>,
    pub right: Option<Battery>,
    pub case: Option<Battery>,
    /// The case only reports while a bud is docked, so its level is kept
    /// after the buds come out and marked as stale.
    pub case_stale: bool,
    pub left_in_case: Option<bool>,
    pub right_in_case: Option<bool>,

    pub anc: Option<AncState>,
    pub multipoint: Option<bool>,
    pub on_head: Option<bool>,
    pub speech: Option<bool>,
    pub volume_notifications: Option<bool>,
    pub volume_eq: Option<bool>,
    pub mono: Option<bool>,
    pub gestures: Option<bool>,
    pub balance: Option<VolumeAsymmetry>,
    pub eq: Option<EqBands>,

    pub firmware: Option<[String; 3]>,
    pub show_info: bool,

    pub selected: usize,
    pub message: Option<(String, Instant)>,
    pub quit: bool,

    commands: UnboundedSender<SettingValue>,
}

impl App {
    pub fn new(commands: UnboundedSender<SettingValue>) -> Self {
        Self {
            status: "Starting…".into(),
            device: None,
            present: false,
            connected: false,
            left: None,
            right: None,
            case: None,
            case_stale: false,
            left_in_case: None,
            right_in_case: None,
            anc: None,
            multipoint: None,
            on_head: None,
            speech: None,
            volume_notifications: None,
            volume_eq: None,
            mono: None,
            gestures: None,
            balance: None,
            eq: None,
            firmware: None,
            show_info: false,
            selected: 0,
            message: None,
            quit: false,
            commands,
        }
    }

    pub fn apply(&mut self, event: LinkEvent) {
        match event {
            LinkEvent::Status(s) => self.status = s,
            LinkEvent::Device { name, address } => {
                self.device = Some((name, address));
                self.present = true;
            }
            LinkEvent::Absent => self.present = false,
            LinkEvent::Connected => {
                self.connected = true;
                self.status = "Connected".into();
            }
            LinkEvent::Disconnected => {
                self.connected = false;
                // Battery is only known through the session; keep the last
                // values on screen but drop the live-only placement.
                self.left_in_case = None;
                self.right_in_case = None;
            }
            LinkEvent::Runtime(info) => self.apply_runtime(info),
            LinkEvent::Setting(value) => self.apply_setting(value),
            LinkEvent::Firmware(info) => {
                let fw = info.firmware.unwrap_or_default();
                let v = |f: Option<FirmwareVersion>| {
                    f.map(|f| f.version_string).unwrap_or_else(|| "unknown".into())
                };
                self.firmware = Some([v(fw.left), v(fw.right), v(fw.case)]);
            }
            LinkEvent::Error(e) => self.flash(e),
        }
    }

    fn apply_runtime(&mut self, info: RuntimeInfo) {
        if let Some(p) = info.placement {
            self.left_in_case = Some(p.left_bud_in_case);
            self.right_in_case = Some(p.right_bud_in_case);
        }
        if let Some(b) = info.battery_info {
            self.left = Battery::from_wire(b.left);
            self.right = Battery::from_wire(b.right);
            match Battery::from_wire(b.case) {
                Some(case) => {
                    self.case = Some(case);
                    self.case_stale = false;
                }
                None => self.case_stale = self.case.is_some(),
            }
        }
    }

    fn apply_setting(&mut self, value: SettingValue) {
        match value {
            SettingValue::CurrentAncrState(v) => self.anc = Some(v),
            SettingValue::MultipointEnable(v) => self.multipoint = Some(v),
            SettingValue::OhdEnable(v) => self.on_head = Some(v),
            SettingValue::SpeechDetection(v) => self.speech = Some(v),
            SettingValue::VolumeExposureNotifications(v) => self.volume_notifications = Some(v),
            SettingValue::VolumeEqEnable(v) => self.volume_eq = Some(v),
            SettingValue::SumToMono(v) => self.mono = Some(v),
            SettingValue::GestureEnable(v) => self.gestures = Some(v),
            SettingValue::VolumeAsymmetry(v) => self.balance = Some(v),
            SettingValue::CurrentUserEq(v) => self.eq = Some(v),
            _ => {}
        }
    }

    pub fn flash(&mut self, msg: impl Into<String>) {
        self.message = Some((msg.into(), Instant::now()));
    }

    pub fn row(&self) -> Row {
        ROWS[self.selected]
    }

    pub fn up(&mut self) {
        self.selected = self.selected.checked_sub(1).unwrap_or(ROWS.len() - 1);
    }

    pub fn down(&mut self) {
        self.selected = (self.selected + 1) % ROWS.len();
    }

    /// Enter/space: toggle a switch, or step ANC forward.
    pub fn activate(&mut self) {
        match self.row() {
            Row::Anc => self.step_anc(true),
            Row::Balance | Row::Eq(_) => {}
            row => self.toggle(row),
        }
    }

    /// Left/right on the current row.
    pub fn adjust(&mut self, forward: bool) {
        match self.row() {
            Row::Anc => self.step_anc(forward),
            Row::Balance => {
                let Some(b) = self.balance else { return };
                let step = if forward { BALANCE_STEP } else { -BALANCE_STEP };
                let v = VolumeAsymmetry::from_normalized(b.value() + step);
                self.write(SettingValue::VolumeAsymmetry(v), |a| a.balance = Some(v));
            }
            Row::Eq(band) => {
                let Some(mut eq) = self.eq else { return };
                let step = if forward { EQ_STEP } else { -EQ_STEP };
                let value = eq_band(&eq, band) + step;
                set_band(&mut eq, band, value);
                self.write(SettingValue::CurrentUserEq(eq), |a| a.eq = Some(eq));
            }
            row => {
                // On a switch, right means on and left means off.
                if self.switch(row) != Some(forward) {
                    self.toggle(row);
                }
            }
        }
    }

    /// `r`: reset the balance or the whole EQ to flat.
    pub fn reset(&mut self) {
        match self.row() {
            Row::Balance => {
                let v = VolumeAsymmetry::from_normalized(0);
                self.write(SettingValue::VolumeAsymmetry(v), |a| a.balance = Some(v));
            }
            Row::Eq(_) => {
                let eq = EqBands::default();
                self.write(SettingValue::CurrentUserEq(eq), |a| a.eq = Some(eq));
            }
            _ => {}
        }
    }

    pub fn set_anc(&mut self, state: AncState) {
        self.write(SettingValue::CurrentAncrState(state), |a| a.anc = Some(state));
    }

    fn step_anc(&mut self, forward: bool) {
        let current = self.anc.unwrap_or(AncState::Off);
        let i = ANC_ORDER.iter().position(|s| *s == current).unwrap_or(0);
        let n = ANC_ORDER.len();
        let next = if forward { (i + 1) % n } else { (i + n - 1) % n };
        self.set_anc(ANC_ORDER[next]);
    }

    pub fn switch(&self, row: Row) -> Option<bool> {
        match row {
            Row::Multipoint => self.multipoint,
            Row::OnHead => self.on_head,
            Row::Speech => self.speech,
            Row::VolumeNotifications => self.volume_notifications,
            Row::VolumeEq => self.volume_eq,
            Row::Mono => self.mono,
            Row::Gestures => self.gestures,
            _ => None,
        }
    }

    fn toggle(&mut self, row: Row) {
        let Some(on) = self.switch(row) else { return };
        let v = !on;
        let (value, apply): (SettingValue, fn(&mut App, bool)) = match row {
            Row::Multipoint => (SettingValue::MultipointEnable(v), |a, v| a.multipoint = Some(v)),
            Row::OnHead => (SettingValue::OhdEnable(v), |a, v| a.on_head = Some(v)),
            Row::Speech => (SettingValue::SpeechDetection(v), |a, v| a.speech = Some(v)),
            Row::VolumeNotifications => (
                SettingValue::VolumeExposureNotifications(v),
                |a, v| a.volume_notifications = Some(v),
            ),
            Row::VolumeEq => (SettingValue::VolumeEqEnable(v), |a, v| a.volume_eq = Some(v)),
            Row::Mono => (SettingValue::SumToMono(v), |a, v| a.mono = Some(v)),
            Row::Gestures => (SettingValue::GestureEnable(v), |a, v| a.gestures = Some(v)),
            _ => return,
        };
        self.write(value, |a| apply(a, v));
    }

    /// Sends a write and shows it straight away; the buds echo the change
    /// back through the settings stream, which confirms or corrects it.
    fn write(&mut self, value: SettingValue, optimistic: impl FnOnce(&mut App)) {
        if !self.connected {
            self.flash("Not connected");
            return;
        }
        if self.commands.send(value).is_ok() {
            optimistic(self);
        }
    }
}

pub fn eq_band(eq: &EqBands, band: usize) -> f32 {
    match band {
        0 => eq.low_bass(),
        1 => eq.bass(),
        2 => eq.mid(),
        3 => eq.treble(),
        _ => eq.upper_treble(),
    }
}

fn set_band(eq: &mut EqBands, band: usize, value: f32) {
    match band {
        0 => eq.set_low_bass(value),
        1 => eq.set_bass(value),
        2 => eq.set_mid(value),
        3 => eq.set_treble(value),
        _ => eq.set_upper_treble(value),
    }
}
