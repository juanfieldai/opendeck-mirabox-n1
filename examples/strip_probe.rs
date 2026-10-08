//! Full-LCD calibration probe for MSD NEO 0b00:1004 firmware V3.MSD-NEO.02.011.
//!
//! Writes one 450x85 JPEG at mirajazz index 15 (BAT wire slot 16), never three tiles.
//! The border and corner markers expose cropping and orientation. This is a calibrated
//! canvas, not proof of the manufacturer's native pixel resolution.
//!
//!     cargo run --example strip_probe
//! Close OpenDeck first. Ctrl-C to stop.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use image::{DynamicImage, Rgb, RgbImage};
use mirajazz::{
    device::{Device, DeviceQuery, list_devices},
    error::MirajazzError,
    types::{ImageFormat, ImageMirroring, ImageMode, ImageRotation},
};

const QUERY: DeviceQuery = DeviceQuery::new(65440, 1, 0x0b00, 0x1004);
const INPUT_KEY_COUNT: usize = 17;
const ENCODER_COUNT: usize = 1;
static MODE_DONE: AtomicBool = AtomicBool::new(false);

fn format() -> ImageFormat {
    ImageFormat {
        mode: ImageMode::JPEG,
        size: (450, 85),
        rotation: ImageRotation::Rot0,
        mirror: ImageMirroring::None,
    }
}

fn calibration_image() -> DynamicImage {
    DynamicImage::ImageRgb8(RgbImage::from_fn(450, 85, |x, y| {
        if x < 2 || x >= 448 || y < 2 || y >= 83 {
            Rgb([255, 255, 255])
        } else if x < 24 && y < 24 {
            Rgb([255, 0, 0])
        } else if x >= 426 && y < 24 {
            Rgb([0, 255, 0])
        } else if x < 24 && y >= 61 {
            Rgb([0, 90, 255])
        } else if x >= 426 && y >= 61 {
            Rgb([255, 255, 0])
        } else {
            Rgb([17, 24, 39])
        }
    }))
}

async fn session() -> Result<(), MirajazzError> {
    let devs = list_devices(&[QUERY]).await?;
    let dev = match devs.into_iter().next() {
        Some(d) => d,
        None => return Ok(()),
    };

    eprintln!("[session] connecting serial={:?}", dev.serial_number);
    let device = Device::connect(&dev, 3, INPUT_KEY_COUNT, ENCODER_COUNT).await?;
    device.set_brightness(100).await?;
    if !MODE_DONE.swap(true, Ordering::SeqCst) {
        device.set_mode(3).await?;
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    loop {
        device.set_button_image(15, format(), calibration_image()).await?;
        device.flush().await?;
        tokio::time::sleep(Duration::from_secs(2)).await;
        device.keep_alive().await?;
    }
}

#[tokio::main]
async fn main() {
    eprintln!("Full-LCD probe, calibrated canvas 450x85. Ctrl-C to stop.");
    loop {
        if let Err(e) = session().await {
            eprintln!("[session] error: {e}");
        }
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
}
