//! Raw declarations for the Objective-C shim in `macos/btshim.m`.

use std::os::raw::{c_char, c_int};

pub const BT_ADDR_LEN: usize = 18;
pub const BT_NAME_LEN: usize = 64;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct bt_device_t {
    pub name: [c_char; BT_NAME_LEN],
    pub addr: [c_char; BT_ADDR_LEN],
    pub connected: bool,
}

#[repr(C)]
pub struct bt_chan {
    _opaque: [u8; 0],
}

unsafe extern "C" {
    pub fn bt_init();
    pub fn bt_run_loop() -> !;
    pub fn bt_list_devices(out: *mut bt_device_t, max: i32) -> i32;
    pub fn bt_device_connected(addr: *const c_char) -> bool;
    pub fn bt_rfcomm_channel_for_uuid(addr: *const c_char, uuid16: *const u8) -> i32;
    pub fn bt_rfcomm_open(addr: *const c_char, channel_id: u8, err: *mut i32) -> *mut bt_chan;
    pub fn bt_l2cap_open(addr: *const c_char, psm: u16, err: *mut i32) -> *mut bt_chan;
    pub fn bt_send(chan: *mut bt_chan, buf: *const u8, len: usize) -> i32;
    pub fn bt_recv(chan: *mut bt_chan, buf: *mut u8, len: usize, timeout_ms: c_int) -> i32;
    pub fn bt_mtu(chan: *mut bt_chan) -> u32;
    pub fn bt_close(chan: *mut bt_chan);
}
