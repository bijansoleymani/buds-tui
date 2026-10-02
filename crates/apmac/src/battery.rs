//! Battery and identity, from what macOS already reports.
//!
//! On Linux buds-tui decodes these from AACP notifications and from Apple's
//! BLE proximity advertisements. Neither is open to us here, but macOS holds
//! the AACP session itself and publishes the results, so this reads them back
//! out of `system_profiler`.
//!
//! The JSON form is used rather than the text form because the text form's
//! labels vary by device kind ("Battery Level" for one bud, "Left Battery
//! Level" for two), whereas the keys do not.

use std::io;
use std::process::Command;

use serde_json::Value;

/// A Bluetooth audio device as macOS describes it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DeviceInfo {
    pub name: String,
    pub address: String,
    pub connected: bool,
    pub vendor_id: Option<u16>,
    pub product_id: Option<u16>,
    pub firmware: Option<String>,
    pub serial: Option<String>,
    pub rssi: Option<i32>,
    /// Single-battery devices report their level here.
    pub battery: Option<u8>,
    pub battery_left: Option<u8>,
    pub battery_right: Option<u8>,
    pub battery_case: Option<u8>,
}

impl DeviceInfo {
    /// True for Apple's vendor id, which is what AirPods and Beats report.
    pub fn is_apple(&self) -> bool {
        self.vendor_id == Some(0x004C)
    }

    /// The lowest level across whatever this device reports, which is what a
    /// status line wants to show.
    pub fn lowest_battery(&self) -> Option<u8> {
        [
            self.battery,
            self.battery_left,
            self.battery_right,
            self.battery_case,
        ]
        .into_iter()
        .flatten()
        .min()
    }
}

/// "99%" -> 99
fn percent(value: Option<&Value>) -> Option<u8> {
    value?.as_str()?.trim_end_matches('%').parse().ok()
}

/// "0x2024" -> 0x2024
fn hex_id(value: Option<&Value>) -> Option<u16> {
    let s = value?.as_str()?.trim();
    u16::from_str_radix(s.strip_prefix("0x").unwrap_or(s), 16).ok()
}

fn text(value: Option<&Value>) -> Option<String> {
    Some(value?.as_str()?.to_string())
}

fn parse_device(name: &str, fields: &Value, connected: bool) -> DeviceInfo {
    DeviceInfo {
        name: name.to_string(),
        address: text(fields.get("device_address")).unwrap_or_default(),
        connected,
        vendor_id: hex_id(fields.get("device_vendorID")),
        product_id: hex_id(fields.get("device_productID")),
        firmware: text(fields.get("device_firmwareVersion")),
        serial: text(fields.get("device_serialNumber")),
        rssi: fields
            .get("device_rssi")
            .and_then(|v| v.as_i64().or_else(|| v.as_str()?.parse().ok()))
            .map(|v| v as i32),
        battery: percent(fields.get("device_batteryLevel")),
        battery_left: percent(fields.get("device_batteryLevelLeft")),
        battery_right: percent(fields.get("device_batteryLevelRight")),
        battery_case: percent(fields.get("device_batteryLevelCase")),
    }
}

/// Pulls the device lists out of the report. Each entry is a single-key object
/// whose key is the device name.
fn collect(list: Option<&Value>, connected: bool, out: &mut Vec<DeviceInfo>) {
    let Some(entries) = list.and_then(|v| v.as_array()) else {
        return;
    };
    for entry in entries {
        let Some(map) = entry.as_object() else {
            continue;
        };
        for (name, fields) in map {
            out.push(parse_device(name, fields, connected));
        }
    }
}

/// Parses a `system_profiler -json SPBluetoothDataType` report.
pub fn parse_report(json: &str) -> io::Result<Vec<DeviceInfo>> {
    let root: Value = serde_json::from_str(json).map_err(io::Error::other)?;
    let mut devices = Vec::new();
    let sections = root
        .get("SPBluetoothDataType")
        .and_then(|v| v.as_array())
        .map(|v| v.as_slice())
        .unwrap_or_default();
    for section in sections {
        collect(section.get("device_connected"), true, &mut devices);
        collect(section.get("device_not_connected"), false, &mut devices);
    }
    Ok(devices)
}

/// Every paired Bluetooth device macOS knows about.
///
/// `system_profiler` takes a moment, so callers should poll this on a timer
/// rather than per frame.
pub fn devices() -> io::Result<Vec<DeviceInfo>> {
    let out = Command::new("/usr/sbin/system_profiler")
        .args(["-json", "SPBluetoothDataType"])
        .output()?;
    if !out.status.success() {
        return Err(io::Error::other(format!(
            "system_profiler failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    parse_report(&String::from_utf8_lossy(&out.stdout))
}

/// Connected Apple audio devices, which is what the AirPods screen wants.
pub fn connected_airpods() -> io::Result<Vec<DeviceInfo>> {
    Ok(devices()?
        .into_iter()
        .filter(|d| d.connected && d.is_apple())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Trimmed from a real report on macOS 26.5.1.
    const REPORT: &str = r#"{
      "SPBluetoothDataType": [{
        "device_connected": [{
          "AirPods Pro #2": {
            "device_address": "98:1C:A2:BF:ED:74",
            "device_batteryLevelLeft": "99%",
            "device_batteryLevelRight": "100%",
            "device_firmwareVersion": "9A348",
            "device_productID": "0x2024",
            "device_rssi": -59,
            "device_serialNumber": "KGXNHXDGH2",
            "device_vendorID": "0x004C"
          }
        }],
        "device_not_connected": [{
          "Bijan's Pixel Buds Pro 2": {
            "device_address": "FC:91:5D:6F:02:9F",
            "device_productID": "0x3005",
            "device_vendorID": "0x00E0"
          }
        }]
      }]
    }"#;

    #[test]
    fn reads_per_bud_battery_and_identity() {
        let devices = parse_report(REPORT).unwrap();
        let pods = devices.iter().find(|d| d.name == "AirPods Pro #2").unwrap();
        assert!(pods.connected);
        assert!(pods.is_apple());
        assert_eq!(pods.battery_left, Some(99));
        assert_eq!(pods.battery_right, Some(100));
        assert_eq!(pods.battery_case, None);
        assert_eq!(pods.product_id, Some(0x2024));
        assert_eq!(pods.firmware.as_deref(), Some("9A348"));
        assert_eq!(pods.rssi, Some(-59));
        assert_eq!(pods.lowest_battery(), Some(99));
    }

    #[test]
    fn separates_connected_from_paired() {
        let devices = parse_report(REPORT).unwrap();
        let buds = devices
            .iter()
            .find(|d| d.name.contains("Pixel Buds"))
            .unwrap();
        assert!(!buds.connected);
        assert!(!buds.is_apple());
        assert_eq!(buds.lowest_battery(), None);
    }

    #[test]
    fn survives_an_empty_or_odd_report() {
        assert!(parse_report("{}").unwrap().is_empty());
        assert!(
            parse_report(r#"{"SPBluetoothDataType":[]}"#)
                .unwrap()
                .is_empty()
        );
        assert!(parse_report("not json").is_err());
    }
}
