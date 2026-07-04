use mirajazz::{
    device::DeviceQuery,
    types::{HidDeviceInfo, ImageFormat, ImageMirroring, ImageMode, ImageRotation},
};

// Must be unique between all the plugins, 2 characters long and match `DeviceNamespace` field in `manifest.json`
pub const DEVICE_NAMESPACE: &str = "n1";

// Mirabox N1: 15 LCD keys arranged like a numpad — 5 rows x 3 columns.
// Encoders (3): 0 = knob (rotate + press), 1 = extra button A, 2 = extra button B.
pub const ROW_COUNT: usize = 5;
pub const COL_COUNT: usize = 3;
pub const KEY_COUNT: usize = ROW_COUNT * COL_COUNT;
pub const ENCODER_COUNT: usize = 3;

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

    pub fn image_format(&self) -> ImageFormat {
        // Determined on hardware: N1 key LCDs are 108x104 (landscape), upright, no mirror.
        ImageFormat {
            mode: ImageMode::JPEG,
            size: (108, 104),
            rotation: ImageRotation::Rot0,
            mirror: ImageMirroring::None,
        }
    }

    /// Image format for the screen-strip segments (one slot per encoder). The strip shares the
    /// keys' image protocol; segments live at device indices KEY_COUNT + encoder_position.
    /// 80x80 centers each segment over its column (determined on hardware).
    pub fn encoder_image_format(&self) -> ImageFormat {
        ImageFormat {
            mode: ImageMode::JPEG,
            size: (80, 80),
            rotation: ImageRotation::Rot0,
            mirror: ImageMirroring::None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CandidateDevice {
    pub id: String,
    pub dev: HidDeviceInfo,
    pub kind: Kind,
}
