//! The slice of CoreAudio and CoreFoundation this crate needs.
//!
//! Hand-declared rather than pulled from a bindings crate: it is a handful of
//! entry points, and the interesting selectors are undocumented anyway.

use std::ffi::c_void;

pub type AudioObjectID = u32;
pub type OSStatus = i32;

pub const K_AUDIO_OBJECT_SYSTEM_OBJECT: AudioObjectID = 1;
pub const K_AUDIO_OBJECT_PROPERTY_SCOPE_GLOBAL: u32 = u32::from_be_bytes(*b"glob");
pub const K_AUDIO_OBJECT_PROPERTY_ELEMENT_MAIN: u32 = 0;
pub const K_AUDIO_OBJECT_PROPERTY_NAME: u32 = u32::from_be_bytes(*b"lnam");
pub const K_AUDIO_HARDWARE_PROPERTY_DEVICES: u32 = u32::from_be_bytes(*b"dev#");

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct AudioObjectPropertyAddress {
    pub selector: u32,
    pub scope: u32,
    pub element: u32,
}

#[link(name = "CoreAudio", kind = "framework")]
unsafe extern "C" {
    pub fn AudioObjectHasProperty(
        id: AudioObjectID,
        addr: *const AudioObjectPropertyAddress,
    ) -> bool;

    pub fn AudioObjectIsPropertySettable(
        id: AudioObjectID,
        addr: *const AudioObjectPropertyAddress,
        settable: *mut bool,
    ) -> OSStatus;

    pub fn AudioObjectGetPropertyDataSize(
        id: AudioObjectID,
        addr: *const AudioObjectPropertyAddress,
        qualifier_size: u32,
        qualifier: *const c_void,
        size: *mut u32,
    ) -> OSStatus;

    pub fn AudioObjectGetPropertyData(
        id: AudioObjectID,
        addr: *const AudioObjectPropertyAddress,
        qualifier_size: u32,
        qualifier: *const c_void,
        size: *mut u32,
        data: *mut c_void,
    ) -> OSStatus;

    pub fn AudioObjectSetPropertyData(
        id: AudioObjectID,
        addr: *const AudioObjectPropertyAddress,
        qualifier_size: u32,
        qualifier: *const c_void,
        size: u32,
        data: *const c_void,
    ) -> OSStatus;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFStringGetLength(s: *const c_void) -> isize;
    fn CFStringGetCString(s: *const c_void, buf: *mut u8, size: isize, encoding: u32) -> bool;
    fn CFRelease(obj: *const c_void);
}

const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;

/// Converts a CFString we own into a Rust string, releasing it.
///
/// # Safety
/// `cfstr` must be a non-null CFStringRef that the caller owns a reference to.
pub unsafe fn cfstring_to_string(cfstr: *const c_void) -> String {
    unsafe {
        // Worst case for UTF-8 is 3 bytes per UTF-16 unit, plus the nul.
        let capacity = CFStringGetLength(cfstr) * 3 + 1;
        let mut buf = vec![0u8; capacity as usize];
        let ok = CFStringGetCString(cfstr, buf.as_mut_ptr(), capacity, K_CF_STRING_ENCODING_UTF8);
        CFRelease(cfstr);
        if !ok {
            return String::new();
        }
        let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        String::from_utf8_lossy(&buf[..end]).into_owned()
    }
}
