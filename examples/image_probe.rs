//! Single fixed-size image test for the Mirabox N1 (native image layer, no set_mode, Rot0).
//!
//! Draws ONE tile size on all keys (full-bleed red frame + white top-left marker) and keeps
//! redrawing + keep-alive, so observation isn't confused by size cycling. Stay on the image layer.
//!
//!     cargo run --example image_probe -- 112      # test size 112 (default)

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use image::{DynamicImage, Rgb, RgbImage};
use mirajazz::{
    device::{Device, DeviceQuery, list_devices},
    error::MirajazzError,
    types::{ImageFormat, ImageMirroring, ImageMode, ImageRotation},
};

const QUERY: DeviceQuery = DeviceQuery::new(65440, 1, 0x6603, 0x1000);
const KEY_COUNT: usize = 15;
const ENCODER_COUNT: usize = 1;

static MODE_DONE: AtomicBool = AtomicBool::new(false);

fn format(w: u32, h: u32) -> ImageFormat {
    ImageFormat {
        mode: ImageMode::JPEG,
        size: (w as usize, h as usize),
        rotation: ImageRotation::Rot0,
        mirror: ImageMirroring::None,
    }
}

/// Full-bleed: green bg, red 3px frame at edges, white top-left corner block.
fn test_image(w: u32, h: u32) -> DynamicImage {
    let b = 3u32;
    let img = RgbImage::from_fn(w, h, |x, y| {
        if x < b || y < b || x >= w - b || y >= h - b {
            Rgb([255, 0, 0])
        } else if x < w / 3 && y < h / 3 {
            Rgb([255, 255, 255])
        } else {
            Rgb([0, 90, 255]) // distinct BLUE background (changed from green to verify fresh draw)
        }
    });
    DynamicImage::ImageRgb8(img)
}

async fn session(w: u32, h: u32) -> Result<(), MirajazzError> {
    let devs = list_devices(&[QUERY]).await?;
    let dev = match devs.into_iter().next() {
        Some(d) => d,
        None => return Ok(()),
    };

    eprintln!("[session] connecting serial={:?}", dev.serial_number);
    let device = Device::connect(&dev, 3, KEY_COUNT + 2, ENCODER_COUNT).await?;
    // Initialize FIRST (this sends DIS+LIG), THEN switch mode, so the init doesn't undo set_mode.
    eprintln!("set_brightness(init) -> {:?}", device.set_brightness(100).await);

    // Only set the mode on the very first connection — set_mode triggers a re-enumeration, so
    // repeating it every reconnect would loop forever.
    let mode: Option<u8> = std::env::var("N1_MODE").ok().and_then(|s| s.parse().ok());
    if let Some(m) = mode {
        if !MODE_DONE.swap(true, Ordering::SeqCst) {
            eprintln!("set_mode({m}) (first connect only) -> {:?}", device.set_mode(m).await);
            tokio::time::sleep(Duration::from_millis(300)).await;
        } else {
            eprintln!("[session] skipping set_mode on reconnect");
        }
    }
    eprintln!("[session] showing fixed SIZE={w}x{h} (Rot0), mode={:?}. Don't touch the device.", mode);

    loop {
        for pos in 0..KEY_COUNT {
            device
                .set_button_image(pos as u8, format(w, h), test_image(w, h))
                .await
                .ok();
        }
        if device.flush().await.is_err() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
        if device.keep_alive().await.is_err() {
            return Ok(());
        }
    }
}

#[tokio::main]
async fn main() {
    let w: u32 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(108);
    let h: u32 = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(w);

    eprintln!("Fixed-size image test SIZE={w}x{h}. Ctrl-C to stop.");
    loop {
        if let Err(e) = session(w, h).await {
            eprintln!("[session] error: {e}");
        }
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
}
