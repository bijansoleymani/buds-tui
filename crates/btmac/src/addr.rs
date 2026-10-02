//! A Bluetooth address, standing in for `bluer::Address` on macOS.

use std::fmt;
use std::str::FromStr;

/// Six bytes, most significant first, like BlueZ and IOBluetooth both print.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Address(pub [u8; 6]);

impl Address {
    pub const fn new(octets: [u8; 6]) -> Self {
        Self(octets)
    }

    /// IOBluetooth hands back and accepts dash-separated addresses.
    pub fn to_dashed(self) -> String {
        let o = self.0;
        format!(
            "{:02x}-{:02x}-{:02x}-{:02x}-{:02x}-{:02x}",
            o[0], o[1], o[2], o[3], o[4], o[5]
        )
    }
}

impl fmt::Display for Address {
    /// Colon-separated upper case, matching what BlueZ shows and what the
    /// rest of buds-tui prints.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let o = self.0;
        write!(
            f,
            "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
            o[0], o[1], o[2], o[3], o[4], o[5]
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidAddress(pub String);

impl fmt::Display for InvalidAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "not a Bluetooth address: {}", self.0)
    }
}

impl std::error::Error for InvalidAddress {}

impl FromStr for Address {
    type Err = InvalidAddress;

    /// Accepts either separator, since IOBluetooth uses dashes and everyone
    /// else uses colons.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.split(['-', ':']).collect();
        if parts.len() != 6 {
            return Err(InvalidAddress(s.to_string()));
        }
        let mut octets = [0u8; 6];
        for (slot, part) in octets.iter_mut().zip(parts) {
            *slot = u8::from_str_radix(part, 16).map_err(|_| InvalidAddress(s.to_string()))?;
        }
        Ok(Self(octets))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_both_separators() {
        let expected = Address([0xfc, 0x91, 0x5d, 0x6f, 0x02, 0x9f]);
        assert_eq!("fc-91-5d-6f-02-9f".parse::<Address>().unwrap(), expected);
        assert_eq!("FC:91:5D:6F:02:9F".parse::<Address>().unwrap(), expected);
    }

    #[test]
    fn round_trips_through_both_renderings() {
        let addr = Address([0xfc, 0x91, 0x5d, 0x6f, 0x02, 0x9f]);
        assert_eq!(addr.to_dashed(), "fc-91-5d-6f-02-9f");
        assert_eq!(addr.to_string(), "FC:91:5D:6F:02:9F");
        assert_eq!(addr.to_dashed().parse::<Address>().unwrap(), addr);
        assert_eq!(addr.to_string().parse::<Address>().unwrap(), addr);
    }

    #[test]
    fn rejects_malformed() {
        assert!("".parse::<Address>().is_err());
        assert!("fc-91-5d-6f-02".parse::<Address>().is_err());
        assert!("fc-91-5d-6f-02-zz".parse::<Address>().is_err());
    }
}
