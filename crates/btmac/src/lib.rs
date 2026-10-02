//! Classic-Bluetooth RFCOMM and L2CAP channels on macOS, over IOBluetooth.
//!
//! Stands in for the parts of `bluer` that buds-tui uses, which is BlueZ-only.
//! What it covers is deliberately narrow: enumerate paired devices, look up a
//! service's RFCOMM channel in SDP, and open a channel as an async byte stream.
//!
//! # The main thread
//!
//! IOBluetooth will only open a channel from the main thread, and only
//! delivers its delegate callbacks to a running run loop. A thread of our own
//! running its own run loop is not enough — `openRFCOMMChannelSync` still
//! fails with `kIOReturnError`. So a program using this crate has to give its
//! main thread to [`run_main_loop`] and do everything else elsewhere.

#![cfg(target_os = "macos")]

use std::ffi::{CStr, CString};
use std::io;

mod addr;
mod channel;
mod ffi;

pub use addr::{Address, InvalidAddress};
pub use channel::Channel;

/// The Maestro protocol, as advertised by Pixel Buds Pro / Pro 2.
pub const MAESTRO_UUID: [u8; 16] = [
    0x25, 0xe9, 0x7f, 0xf7, 0x24, 0xce, 0x4c, 0x4c, 0x89, 0x51, 0xf7, 0x64, 0xa7, 0x08, 0xf7, 0xb5,
];

/// Apple's AACP control channel. Present on AirPods, but macOS keeps the one
/// usable session for itself: the channel opens only in a brief window right
/// after connection, and the device never answers a second session. Kept here
/// because the AirPods code path still has to name it.
pub const AACP_PSM: u16 = 0x1001;

/// A paired device, as macOS sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub name: String,
    pub addr: Address,
    pub connected: bool,
}

/// Prepares the shim. Cheap and idempotent.
pub fn init() {
    // SAFETY: the shim guards against repeat calls.
    unsafe { ffi::bt_init() };
}

/// Hands the calling thread to a run loop forever. Must be the main thread;
/// see the module docs for why.
pub fn run_main_loop() -> ! {
    init();
    // SAFETY: diverges by contract.
    unsafe { ffi::bt_run_loop() }
}

fn c_string(addr: Address) -> CString {
    CString::new(addr.to_dashed()).expect("address rendering has no interior nul")
}

fn to_string(raw: &[std::os::raw::c_char]) -> String {
    // SAFETY: the shim nul-terminates both fields within their fixed arrays.
    unsafe { CStr::from_ptr(raw.as_ptr()) }
        .to_string_lossy()
        .into_owned()
}

/// Every device macOS has paired, connected or not.
pub fn paired_devices() -> io::Result<Vec<Device>> {
    init();
    const MAX: usize = 64;
    let mut raw = vec![
        ffi::bt_device_t {
            name: [0; ffi::BT_NAME_LEN],
            addr: [0; ffi::BT_ADDR_LEN],
            connected: false,
        };
        MAX
    ];
    // SAFETY: the shim writes at most MAX entries into raw.
    let n = unsafe { ffi::bt_list_devices(raw.as_mut_ptr(), MAX as i32) };
    if n < 0 {
        return Err(io::Error::other(
            "Bluetooth is unavailable: no controller, or permission was denied",
        ));
    }
    Ok(raw[..n as usize]
        .iter()
        .filter_map(|d| {
            Some(Device {
                name: to_string(&d.name),
                addr: to_string(&d.addr).parse().ok()?,
                connected: d.connected,
            })
        })
        .collect())
}

/// Whether the device currently has a baseband link to this Mac.
pub fn is_connected(addr: Address) -> bool {
    init();
    let c = c_string(addr);
    // SAFETY: c outlives the call.
    unsafe { ffi::bt_device_connected(c.as_ptr()) }
}

/// The RFCOMM channel the device advertises for `uuid` in its SDP records.
pub fn rfcomm_channel_for_uuid(addr: Address, uuid: &[u8; 16]) -> io::Result<u8> {
    init();
    let c = c_string(addr);
    // SAFETY: both pointers outlive the call.
    let rc = unsafe { ffi::bt_rfcomm_channel_for_uuid(c.as_ptr(), uuid.as_ptr()) };
    match rc {
        ch if ch >= 0 => Ok(ch as u8),
        -1 => Err(io::Error::new(
            io::ErrorKind::NotFound,
            "device does not advertise that service",
        )),
        -2 => Err(io::Error::other("service advertises no RFCOMM channel")),
        _ => Err(io::Error::new(
            io::ErrorKind::NotFound,
            "unknown device, or it has never been paired",
        )),
    }
}

fn io_error(what: &str, code: i32) -> io::Error {
    io::Error::other(format!("{what} failed (IOReturn {code:#010x})"))
}

/// Opens an RFCOMM channel by channel id.
pub fn open_rfcomm(addr: Address, channel_id: u8) -> io::Result<Channel> {
    init();
    let c = c_string(addr);
    let mut err = 0i32;
    // SAFETY: c outlives the call; a non-null return is an owned handle.
    let raw = unsafe { ffi::bt_rfcomm_open(c.as_ptr(), channel_id, &mut err) };
    if raw.is_null() {
        return Err(io_error("openRFCOMMChannelSync", err));
    }
    Ok(Channel::new(raw))
}

/// Opens an L2CAP channel on `psm`.
pub fn open_l2cap(addr: Address, psm: u16) -> io::Result<Channel> {
    init();
    let c = c_string(addr);
    let mut err = 0i32;
    // SAFETY: c outlives the call; a non-null return is an owned handle.
    let raw = unsafe { ffi::bt_l2cap_open(c.as_ptr(), psm, &mut err) };
    if raw.is_null() {
        return Err(io_error("openL2CAPChannelSync", err));
    }
    Ok(Channel::new(raw))
}

/// The first connected device advertising the Maestro service, preferring
/// `wanted` when it is given. Only connected devices count: opening the TUI
/// should not pull the buds away from a phone.
pub fn find_maestro_device(wanted: Option<Address>) -> io::Result<Option<Device>> {
    let devices = paired_devices()?;
    let candidates: Vec<&Device> = match wanted {
        Some(addr) => devices.iter().filter(|d| d.addr == addr).collect(),
        None => devices.iter().collect(),
    };
    Ok(candidates
        .into_iter()
        .find(|d| d.connected && rfcomm_channel_for_uuid(d.addr, &MAESTRO_UUID).is_ok())
        .cloned())
}
