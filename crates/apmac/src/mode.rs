//! Noise control, through two undocumented CoreAudio properties.
//!
//! buds-tui gets this on Linux by speaking Apple's AACP protocol over L2CAP.
//! That is not available on macOS: AACP allows one session per device and the
//! OS holds it (see `docs/macos.md`). But the OS also *acts* on that session,
//! and exposes the listening mode as a property on the AirPods audio device —
//! so instead of driving the accessory we ask macOS to drive it for us.
//!
//! Both are four-char-code selectors on the audio device:
//!
//!   * `lstm` — the current mode, readable and writable
//!   * `lsms` — a bitmask of the modes the device offers
//!
//! Neither is documented, so a macOS update can take them away. Writes are
//! also acknowledged by macOS's own cache rather than by the AirPods, so a
//! successful write means "macOS accepted it", not "the buds confirmed it".

use std::ffi::c_void;

use crate::ffi::{
    AudioObjectGetPropertyData, AudioObjectGetPropertyDataSize, AudioObjectHasProperty,
    AudioObjectID, AudioObjectIsPropertySettable, AudioObjectPropertyAddress,
    AudioObjectSetPropertyData, K_AUDIO_HARDWARE_PROPERTY_DEVICES,
    K_AUDIO_OBJECT_PROPERTY_ELEMENT_MAIN, K_AUDIO_OBJECT_PROPERTY_NAME,
    K_AUDIO_OBJECT_PROPERTY_SCOPE_GLOBAL, K_AUDIO_OBJECT_SYSTEM_OBJECT, cfstring_to_string,
};

/// `lstm`
const LISTENING_MODE: u32 = u32::from_be_bytes(*b"lstm");
/// `lsms`
const SUPPORTED_MODES: u32 = u32::from_be_bytes(*b"lsms");

/// The noise-control modes, numbered as the `lstm` property numbers them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u32)]
pub enum NoiseMode {
    Off = 1,
    NoiseCancellation = 2,
    Transparency = 3,
    Adaptive = 4,
}

impl NoiseMode {
    pub const ALL: [NoiseMode; 4] = [
        NoiseMode::Off,
        NoiseMode::NoiseCancellation,
        NoiseMode::Transparency,
        NoiseMode::Adaptive,
    ];

    pub fn from_raw(raw: u32) -> Option<Self> {
        match raw {
            1 => Some(NoiseMode::Off),
            2 => Some(NoiseMode::NoiseCancellation),
            3 => Some(NoiseMode::Transparency),
            4 => Some(NoiseMode::Adaptive),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            NoiseMode::Off => "Off",
            NoiseMode::NoiseCancellation => "Noise Cancellation",
            NoiseMode::Transparency => "Transparency",
            NoiseMode::Adaptive => "Adaptive",
        }
    }

    /// Position in the `lsms` bitmask. Modes are numbered from 1, bits from 0.
    fn bit(self) -> u32 {
        1 << (self as u32 - 1)
    }
}

/// An audio device that reports a listening mode, i.e. a connected pair of
/// AirPods (or Beats) that macOS is currently driving.
#[derive(Debug, Clone)]
pub struct Headphones {
    id: AudioObjectID,
    pub name: String,
    pub mode: Option<NoiseMode>,
    pub supported: Vec<NoiseMode>,
}

impl Headphones {
    /// Builds a `Headphones` not backed by a real audio device, so UI code can
    /// be tested without hardware. Writes against it fail with
    /// [`ModeError::Unsupported`], since object id 0 has no properties.
    #[doc(hidden)]
    pub fn synthetic(name: &str, mode: Option<NoiseMode>, supported: Vec<NoiseMode>) -> Self {
        Self {
            id: 0,
            name: name.to_string(),
            mode,
            supported,
        }
    }

    /// Re-reads the current mode from macOS.
    pub fn refresh(&mut self) {
        self.mode = read_u32(self.id, LISTENING_MODE).and_then(NoiseMode::from_raw);
    }

    /// Writes the mode and returns once macOS has accepted the write.
    ///
    /// Acceptance is not the same as the mode changing. macOS answers reads
    /// from its own cache, and right after a write the cache reports the value
    /// that was asked for even when the AirPods go on to ignore it — writing
    /// `Off` while the buds' "Off Listening Mode" setting is disabled does
    /// exactly that. Callers that need the truth should use
    /// [`Headphones::set_mode_confirmed`], or re-read a second later.
    pub fn set_mode(&mut self, mode: NoiseMode) -> Result<(), ModeError> {
        self.write_mode(mode)?;
        self.refresh();
        Ok(())
    }

    /// Writes the mode, then waits for the device to still be in it.
    ///
    /// Polls until `timeout` elapses, so a value the cache reported eagerly
    /// and then reverted is caught and reported as [`ModeError::NotApplied`].
    /// Blocks, so this is for command-line use rather than a redraw loop.
    pub fn set_mode_confirmed(
        &mut self,
        mode: NoiseMode,
        timeout: std::time::Duration,
    ) -> Result<(), ModeError> {
        self.write_mode(mode)?;

        let step = std::time::Duration::from_millis(100);
        let deadline = std::time::Instant::now() + timeout;
        let mut settled = false;
        while std::time::Instant::now() < deadline {
            std::thread::sleep(step);
            self.refresh();
            if self.mode == Some(mode) {
                // Seen once is not enough: the cache reports the requested
                // value first and may revert. Require it to still hold.
                if settled {
                    return Ok(());
                }
                settled = true;
            } else {
                settled = false;
            }
        }
        if self.mode == Some(mode) {
            Ok(())
        } else {
            Err(ModeError::NotApplied(mode))
        }
    }

    /// The write itself, with no opinion about whether it stuck.
    fn write_mode(&self, mode: NoiseMode) -> Result<(), ModeError> {
        let addr = address(LISTENING_MODE);
        // SAFETY: a valid device id and a property address living across the call.
        unsafe {
            if !AudioObjectHasProperty(self.id, &addr) {
                return Err(ModeError::Unsupported);
            }
            let mut settable = false;
            let rc = AudioObjectIsPropertySettable(self.id, &addr, &mut settable);
            if rc != 0 {
                return Err(ModeError::CoreAudio(rc));
            }
            if !settable {
                return Err(ModeError::NotSettable);
            }
            let raw = mode as u32;
            let rc = AudioObjectSetPropertyData(
                self.id,
                &addr,
                0,
                std::ptr::null(),
                size_of::<u32>() as u32,
                (&raw as *const u32).cast::<c_void>(),
            );
            if rc != 0 {
                return Err(ModeError::CoreAudio(rc));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModeError {
    /// The device has no `lstm` property, so macOS is not driving its mode.
    Unsupported,
    /// The property exists but macOS will not let it be written now.
    NotSettable,
    /// CoreAudio returned an OSStatus.
    CoreAudio(i32),
    /// macOS accepted the write but the mode did not change. Seen when
    /// writing `Off` while "Off Listening Mode" is disabled on the AirPods.
    NotApplied(NoiseMode),
}

impl std::fmt::Display for ModeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ModeError::Unsupported => {
                write!(f, "this device does not expose a listening mode")
            }
            ModeError::NotSettable => {
                write!(f, "macOS will not accept a listening-mode change right now")
            }
            ModeError::CoreAudio(rc) => write!(f, "CoreAudio rejected the write (OSStatus {rc})"),
            ModeError::NotApplied(NoiseMode::Off) => write!(
                f,
                "macOS accepted Off but the mode did not change — \
                 enable \"Off Listening Mode\" in the AirPods settings"
            ),
            ModeError::NotApplied(mode) => write!(
                f,
                "macOS accepted {} but the mode did not change",
                mode.label()
            ),
        }
    }
}

impl std::error::Error for ModeError {}

fn address(selector: u32) -> AudioObjectPropertyAddress {
    AudioObjectPropertyAddress {
        selector,
        scope: K_AUDIO_OBJECT_PROPERTY_SCOPE_GLOBAL,
        element: K_AUDIO_OBJECT_PROPERTY_ELEMENT_MAIN,
    }
}

fn read_u32(id: AudioObjectID, selector: u32) -> Option<u32> {
    let addr = address(selector);
    // SAFETY: valid id, address and out-pointer all live across the calls.
    unsafe {
        if !AudioObjectHasProperty(id, &addr) {
            return None;
        }
        let mut value: u32 = 0;
        let mut size = size_of::<u32>() as u32;
        let rc = AudioObjectGetPropertyData(
            id,
            &addr,
            0,
            std::ptr::null(),
            &mut size,
            (&mut value as *mut u32).cast::<c_void>(),
        );
        (rc == 0).then_some(value)
    }
}

fn device_name(id: AudioObjectID) -> Option<String> {
    let addr = address(K_AUDIO_OBJECT_PROPERTY_NAME);
    // SAFETY: valid id; CoreAudio writes one retained CFStringRef.
    unsafe {
        let mut cfstr: *const c_void = std::ptr::null();
        let mut size = size_of::<*const c_void>() as u32;
        let rc = AudioObjectGetPropertyData(
            id,
            &addr,
            0,
            std::ptr::null(),
            &mut size,
            (&mut cfstr as *mut *const c_void).cast::<c_void>(),
        );
        if rc != 0 || cfstr.is_null() {
            return None;
        }
        Some(cfstring_to_string(cfstr))
    }
}

fn all_device_ids() -> Vec<AudioObjectID> {
    let addr = address(K_AUDIO_HARDWARE_PROPERTY_DEVICES);
    // SAFETY: the system object is always valid; the buffer is sized first.
    unsafe {
        let mut size = 0u32;
        if AudioObjectGetPropertyDataSize(
            K_AUDIO_OBJECT_SYSTEM_OBJECT,
            &addr,
            0,
            std::ptr::null(),
            &mut size,
        ) != 0
        {
            return Vec::new();
        }
        let count = size as usize / size_of::<AudioObjectID>();
        let mut ids = vec![0 as AudioObjectID; count];
        if AudioObjectGetPropertyData(
            K_AUDIO_OBJECT_SYSTEM_OBJECT,
            &addr,
            0,
            std::ptr::null(),
            &mut size,
            ids.as_mut_ptr().cast::<c_void>(),
        ) != 0
        {
            return Vec::new();
        }
        ids
    }
}

/// Every audio device that reports a listening mode.
///
/// macOS publishes several objects per Bluetooth headset; only the one it is
/// driving carries these properties, so this filters on their presence rather
/// than on the device name.
pub fn headphones() -> Vec<Headphones> {
    all_device_ids()
        .into_iter()
        .filter_map(|id| {
            let mode = read_u32(id, LISTENING_MODE).and_then(NoiseMode::from_raw);
            let mask = read_u32(id, SUPPORTED_MODES);
            // A device with neither property is not one macOS drives.
            if mode.is_none() && mask.is_none() {
                return None;
            }
            let supported = mask
                .map(|m| {
                    NoiseMode::ALL
                        .into_iter()
                        .filter(|mode| m & mode.bit() != 0)
                        .collect()
                })
                .unwrap_or_default();
            Some(Headphones {
                id,
                name: device_name(id).unwrap_or_else(|| "(unnamed device)".into()),
                mode,
                supported,
            })
        })
        .collect()
}

/// The first device macOS is actually driving a listening mode for.
pub fn active() -> Option<Headphones> {
    headphones().into_iter().find(|h| h.mode.is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_numbering_matches_the_property() {
        assert_eq!(NoiseMode::Off as u32, 1);
        assert_eq!(NoiseMode::Adaptive as u32, 4);
        assert_eq!(NoiseMode::from_raw(3), Some(NoiseMode::Transparency));
        assert_eq!(NoiseMode::from_raw(0), None);
        assert_eq!(NoiseMode::from_raw(5), None);
    }

    #[test]
    fn bitmask_decodes_the_observed_value() {
        // 0x07 is what AirPods Pro USB-C reported: Off, NC, Transparency.
        let mask = 0x07u32;
        let got: Vec<_> = NoiseMode::ALL
            .into_iter()
            .filter(|m| mask & m.bit() != 0)
            .collect();
        assert_eq!(
            got,
            vec![
                NoiseMode::Off,
                NoiseMode::NoiseCancellation,
                NoiseMode::Transparency
            ]
        );
    }

    #[test]
    fn dropped_off_write_names_the_setting_to_change() {
        let msg = ModeError::NotApplied(NoiseMode::Off).to_string();
        assert!(msg.contains("Off Listening Mode"), "{msg}");
        let other = ModeError::NotApplied(NoiseMode::Adaptive).to_string();
        assert!(other.contains("Adaptive"), "{other}");
        assert!(!other.contains("Off Listening Mode"), "{other}");
    }

    #[test]
    fn selectors_are_the_expected_four_char_codes() {
        assert_eq!(LISTENING_MODE, 0x6c73746d);
        assert_eq!(SUPPORTED_MODES, 0x6c736d73);
    }
}
