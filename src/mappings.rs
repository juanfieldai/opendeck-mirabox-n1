use mirajazz::{device::DeviceQuery, types::HidDeviceInfo};

// Must be unique between all the plugins, 2 characters long and match `DeviceNamespace` field in `manifest.json`
pub const DEVICE_NAMESPACE: &str = "n1";

// Mirabox N1: 15 LCD keys arranged like a numpad — 5 rows x 3 columns.
// A/B are screenless touch points; the knob is the only encoder.
pub const ROW_COUNT: usize = 5;
pub const COL_COUNT: usize = 3;
pub const KEY_COUNT: usize = ROW_COUNT * COL_COUNT;
pub const TOUCHPOINT_COUNT: usize = 2;
pub const INPUT_KEY_COUNT: usize = KEY_COUNT + TOUCHPOINT_COUNT;
pub const ENCODER_COUNT: usize = 1;
pub const INFOBAR_COUNT: usize = 1;
/// Calibrated full-LCD canvas on MSD NEO firmware V3.MSD-NEO.02.011.
/// Mirajazz index 15 becomes BAT wire slot 16; it is not an encoder display.
pub const LCD_IMAGE_INDEX: u8 = 15;
pub const LCD_SIZE: (u32, u32) = (450, 85);
/// Calibrated on MSD NEO (`0b00:1004`, firmware V3.MSD-NEO.02.011): edge-marker patterns fit
/// exactly at 105x100, upright, no mirror. Upstream used 108x104 for `6603:1000`; Mirabox's SDK says 96x96.
pub const KEY_SIZE: (u32, u32) = (105, 100);

/// Physical arrangement for OpenDeck's editor: A, B and the knob on top, the LCD strip below
/// them, then the 5x3 keypad. OpenDeck builds without `layout` support ignore this field and
/// show their default arrangement; frames then arrive at OpenDeck's default sizes as JPEG.
pub fn editor_layout() -> serde_json::Value {
    let cell = |controller: &str, position: usize| serde_json::json!({ "controller": controller, "position": position });
    let mut rows = vec![
        vec![cell("Keypad", KEY_COUNT), cell("Keypad", KEY_COUNT + 1), cell("Encoder", 0)],
        vec![cell("Infobar", 0)],
    ];
    rows.extend((0..ROW_COUNT).map(|row| (0..COL_COUNT).map(|col| cell("Keypad", row * COL_COUNT + col)).collect()));
    serde_json::json!({
        "rows": rows,
        "keySize": [KEY_SIZE.0, KEY_SIZE.1],
        "infobarSize": [LCD_SIZE.0, LCD_SIZE.1],
        "lossless": true,
    })
}

#[derive(Debug, Clone)]
pub enum Kind {
    N1,
}

// All N1 variants expose their vendor HID interface on usage page 65440 / usage id 1
// (confirmed from the device's HID report descriptor).
const USAGE_PAGE: u16 = 65440;
const USAGE_ID: u16 = 1;

/// A supported device: its USB vendor/product id and the `Kind` that drives it. To support
/// another unit, add a row here and a matching udev rule — `QUERIES` and `Kind::from_vid_pid`
/// are both derived from this table.
pub struct DeviceSpec {
    pub vid: u16,
    pub pid: u16,
    pub kind: Kind,
}

pub const SPECS: &[DeviceSpec] = &[
    DeviceSpec { vid: 0x6603, pid: 0x1000, kind: Kind::N1 },
    DeviceSpec { vid: 0x0300, pid: 0x3007, kind: Kind::N1 },
    DeviceSpec { vid: 0x0b00, pid: 0x1004, kind: Kind::N1 },
];

/// HID queries for every supported device, derived from `SPECS`.
pub const QUERIES: [DeviceQuery; SPECS.len()] = {
    const SEED: DeviceQuery = DeviceQuery::new(USAGE_PAGE, USAGE_ID, 0, 0);

    let mut queries = [SEED; SPECS.len()];
    let mut i = 0;
    while i < SPECS.len() {
        queries[i] = DeviceQuery::new(USAGE_PAGE, USAGE_ID, SPECS[i].vid, SPECS[i].pid);
        i += 1;
    }
    queries
};

impl Kind {
    /// Matches devices VID+PID pairs to correct kinds
    pub fn from_vid_pid(vid: u16, pid: u16) -> Option<Self> {
        SPECS
            .iter()
            .find(|spec| spec.vid == vid && spec.pid == pid)
            .map(|spec| spec.kind.clone())
    }

    /// There is no point relying on manufacturer/device names reported by the USB stack,
    /// so we return custom names for all the kinds of devices
    pub fn human_name(&self) -> String {
        match &self {
            Self::N1 => "Mirabox N1",
        }
        .to_string()
    }

    /// Returns protocol version for device
    pub fn protocol_version(&self) -> usize {
        match self {
            // N1 has a unique serial and a 1024-byte output endpoint, matching the v3 generation.
            // If connecting fails, try version 2.
            Self::N1 => 3,
        }
    }

    /// Some devices boot into a different layer (the N1 starts as a numpad) and must be switched
    /// into their "PC / stream-dock" mode before they display host images. `set_mode(3)` does this
    /// for the N1. Must be sent AFTER the device is initialized.
    pub fn mode(&self) -> Option<u8> {
        match self {
            Self::N1 => Some(3),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CandidateDevice {
    pub id: String,
    pub dev: HidDeviceInfo,
    pub kind: Kind,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// OpenDeck discards a layout that misses or repeats any registered control.
    #[test]
    fn editor_layout_places_every_registered_control_exactly_once() {
        let layout = editor_layout();
        let mut placed: Vec<(String, u64)> = layout["rows"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|row| row.as_array().unwrap().iter())
            .map(|cell| (cell["controller"].as_str().unwrap().to_owned(), cell["position"].as_u64().unwrap()))
            .collect();
        placed.sort();
        let mut registered: Vec<(String, u64)> = [("Keypad", INPUT_KEY_COUNT), ("Encoder", ENCODER_COUNT), ("Infobar", INFOBAR_COUNT)]
            .into_iter()
            .flat_map(|(controller, count)| (0..count as u64).map(move |position| (controller.to_owned(), position)))
            .collect();
        registered.sort();
        assert_eq!(placed, registered);
    }
}
