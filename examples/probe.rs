//! Hardware bring-up probe for the Mirabox N1.
//!
//! Connects to the device directly via mirajazz (no OpenDeck needed) and prints every raw
//! (input, state) pair the device reports, so we can map physical keys / knob / buttons to codes.
//!
//! The N1 re-enumerates on the USB bus after the init handshake, so this probe auto-reconnects
//! in a loop (mirroring what the real plugin's watcher does). Run with the device plugged in:
//!     cargo run --example probe
//! Stop with Ctrl-C (or kill the background process).

use std::time::Duration;

use mirajazz::{
    device::{Device, DeviceQuery, list_devices},
    error::MirajazzError,
    types::DeviceInput,
};

const QUERY: DeviceQuery = DeviceQuery::new(65440, 1, 0x6603, 0x1000);

// Input state includes the 15 LCD keys and two screenless auxiliary buttons.
const INPUT_KEY_COUNT: usize = 17;
const ENCODER_COUNT: usize = 1;

fn log_input(input: u8, state: u8) -> Result<DeviceInput, MirajazzError> {
    // stderr is unbuffered, so lines survive even if the process is killed.
    eprintln!(">> INPUT code=0x{:02x} ({:>3}) state={}", input, input, state);
    Err(MirajazzError::BadData)
}

/// One connect+read session. Returns Ok(()) on clean disconnect so the outer loop reconnects.
async fn session() -> Result<(), MirajazzError> {
    let devs = list_devices(&[QUERY]).await?;
    let dev = match devs.into_iter().next() {
        Some(d) => d,
        None => return Ok(()), // nothing connected yet
    };

    eprintln!(
        "[session] connecting vid={:04x} pid={:04x} serial={:?}",
        dev.vendor_id, dev.product_id, dev.serial_number
    );

    let device = Device::connect(&dev, 3, INPUT_KEY_COUNT, ENCODER_COUNT).await?;
    device.set_brightness(50).await.ok();
    device.clear_all_button_images().await.ok();
    device.flush().await.ok();
    eprintln!("[session] connected, reading input — press keys/knob/buttons now");

    let reader = device.get_reader(log_input);
    loop {
        match reader.read(None).await {
            Ok(updates) => {
                for u in updates {
                    eprintln!("decoded update: {:?}", u);
                }
            }
            Err(MirajazzError::BadData) => {} // expected from log_input
            Err(e) => {
                eprintln!("[session] read ended: {e}");
                return Ok(());
            }
        }
    }
}

#[tokio::main]
async fn main() {
    eprintln!("Probe started. Auto-reconnects across re-enumeration. Ctrl-C to stop.");
    loop {
        if let Err(e) = session().await {
            eprintln!("[session] error: {e}");
        }
        // Give the device a moment to re-enumerate before scanning again.
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
}
