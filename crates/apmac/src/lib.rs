//! AirPods noise control and battery on macOS.
//!
//! buds-tui gets all of this on Linux by speaking Apple's AACP protocol over
//! an L2CAP channel. That is closed to us on macOS: AACP allows one session
//! per device and the OS holds it — the channel can only be opened in a brief
//! window right after connection, and the AirPods never answer a second
//! session. `docs/macos.md` has the measurements.
//!
//! The way through is to stop trying to drive the accessory and instead read
//! and steer the OS that is already driving it:
//!
//!   * [`mode`] reads and writes the listening mode through two undocumented
//!     CoreAudio properties on the AirPods audio device.
//!   * [`battery`] reads per-bud battery and identity out of
//!     `system_profiler`.
//!
//! This covers noise control and battery, which is the most-used part of the
//! AirPods screen, but not the rest: conversation awareness, ear detection,
//! stem configuration and the other AACP settings have no equivalent here.
//! Being undocumented, the CoreAudio properties can also change or vanish in
//! a macOS update, and a successful write means macOS accepted the change
//! rather than that the AirPods acknowledged it.

#![cfg(target_os = "macos")]

pub mod battery;
mod ffi;
pub mod mode;

pub use battery::{DeviceInfo, connected_airpods, devices};
pub use mode::{Headphones, ModeError, NoiseMode, active, headphones};
