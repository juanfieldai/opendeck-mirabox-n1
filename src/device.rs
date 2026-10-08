use data_url::DataUrl;
use image::{DynamicImage, RgbImage, codecs::jpeg::JpegEncoder, imageops::FilterType, load_from_memory_with_format};
use mirajazz::{device::Device, error::MirajazzError, state::DeviceStateUpdate};
use openaction::{OUTBOUND_EVENT_MANAGER, SetImageEvent};
use std::{
    collections::HashMap,
    sync::LazyLock,
    time::{Duration, SystemTime},
};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::{
    DEVICES, TOKENS,
    mappings::{COL_COUNT, CandidateDevice, ENCODER_COUNT, INFOBAR_COUNT, INPUT_KEY_COUNT, KEY_COUNT, KEY_SIZE, LCD_IMAGE_INDEX, LCD_SIZE, ROW_COUNT, TOUCHPOINT_COUNT, editor_layout},
};

/// Hardware JPEG for keys and the full LCD: 4:4:4 (image crate default), q95 accepted on hardware.
const JPEG_QUALITY: u8 = 95;

/// Keep-alive loop period.
const KEEPALIVE_PERIOD: Duration = Duration::from_secs(2);

/// If the real-time gap between two ticks exceeds this, we assume the host was suspended (the
/// monotonic timer driving the loop is frozen during sleep while the system clock keeps real
/// time) and fully reconnect the device, which the N1 firmware needs after a resume.
const RESUME_GAP: Duration = Duration::from_secs(30);

/// Connects to a device and runs the init sequence that switches it into image mode.
async fn connect_and_init(candidate: &CandidateDevice) -> Result<Device, MirajazzError> {
    let device = connect(candidate).await?;

    // Initialize first (set_brightness triggers it), then switch the device into its image
    // mode — doing it in this order keeps the init sequence from undoing the mode switch.
    device.set_brightness(50).await?;
    if let Some(mode) = candidate.kind.mode() {
        device.set_mode(mode).await?;
    }
    device.clear_all_button_images().await?;
    device.flush().await?;

    Ok(device)
}

/// Resolves once a resume-from-suspend is detected: a wall-clock gap between ticks far larger
/// than the polling period (the monotonic timer is frozen while the host sleeps).
async fn wait_for_resume() {
    let mut last_tick = SystemTime::now();

    loop {
        tokio::time::sleep(KEEPALIVE_PERIOD).await;

        let now = SystemTime::now();
        let elapsed = now.duration_since(last_tick).unwrap_or(Duration::ZERO);
        last_tick = now;

        if elapsed >= RESUME_GAP {
            return;
        }
    }
}

/// Initializes a device and listens for events, fully reconnecting after a resume from suspend.
pub async fn device_task(candidate: CandidateDevice, token: CancellationToken) {
    log::info!("Running device task for {:?}", candidate);

    loop {
        let device = match connect_and_init(&candidate).await {
            Ok(device) => device,
            Err(err) => {
                handle_error(&candidate.id, err).await;

                log::error!(
                    "Had error during device init, finishing device task: {:?}",
                    candidate
                );

                return;
            }
        };

        DEVICES.write().await.insert(candidate.id.clone(), device);
        log::info!("Registering device {}", candidate.id);
        if let Some(outbound) = OUTBOUND_EVENT_MANAGER.lock().await.as_mut() {
            // openaction 1.1.5's register_device omits touchpoints and infobars.
            outbound
                .send_event(serde_json::json!({
                    "event": "registerDevice",
                    "payload": {
                        "id": candidate.id,
                        "name": candidate.kind.human_name(),
                        "rows": ROW_COUNT,
                        "columns": COL_COUNT,
                        "encoders": ENCODER_COUNT,
                        "touchpoints": TOUCHPOINT_COUNT,
                        "infobars": INFOBAR_COUNT,
                        "type": 0,
                        "layout": editor_layout()
                    }
                }))
                .await
                .unwrap();
        }
        restore_direct_lcd(&candidate.id).await;

        // After a resume the N1 firmware is back in its default mode and won't deliver input over
        // the existing handle, so we tear everything down and reconnect from scratch — an in-place
        // re-init leaves the stale reader and the firmware's default screen in place.
        let resumed = tokio::select! {
            _ = device_events_task(&candidate) => false,
            _ = device_keepalive_task(&candidate.id, token.clone()) => false,
            _ = wait_for_resume() => true,
            _ = token.cancelled() => false,
        };

        log::info!("Shutting down device {:?}", candidate);

        if let Some(device) = DEVICES.write().await.remove(&candidate.id) {
            device.shutdown().await.ok();
        }

        if resumed && !token.is_cancelled() {
            log::info!("Resumed from suspend, reconnecting device {}", candidate.id);

            // Drop OpenDeck's registration so it re-registers and repaints (clearing the firmware's
            // default screen) once we reconnect on the next loop iteration.
            if let Some(outbound) = OUTBOUND_EVENT_MANAGER.lock().await.as_mut() {
                outbound.deregister_device(candidate.id.clone()).await.ok();
            }

            continue;
        }

        break;
    }

    log::info!("Device task finished for {:?}", candidate);
}

/// The N1 re-enumerates / drops off the bus if the host stops talking to it, so we send a
/// periodic keep-alive ("CONNECT") ping to keep the connection stable.
async fn device_keepalive_task(id: &String, token: CancellationToken) {
    loop {
        tokio::select! {
            _ = tokio::time::sleep(KEEPALIVE_PERIOD) => {}
            _ = token.cancelled() => return,
        }

        let result = {
            let guard = DEVICES.read().await;
            match guard.get(id) {
                Some(device) => device.keep_alive().await,
                None => return,
            }
        };

        if let Err(err) = result {
            handle_error(id, err).await;
            return;
        }
    }
}

/// Handles errors, returning true if should continue, returning false if an error is fatal
pub async fn handle_error(id: &String, err: MirajazzError) -> bool {
    log::error!("Device {} error: {}", id, err);

    // Some errors are not critical and can be ignored without sending disconnected event
    if matches!(err, MirajazzError::ImageError(_) | MirajazzError::BadData) {
        return true;
    }

    log::info!("Deregistering device {}", id);
    if let Some(outbound) = OUTBOUND_EVENT_MANAGER.lock().await.as_mut() {
        outbound.deregister_device(id.clone()).await.unwrap();
    }

    log::info!("Cancelling tasks for device {}", id);
    if let Some(token) = TOKENS.read().await.get(id) {
        token.cancel();
    }

    log::info!("Removing device {} from the list", id);
    DEVICES.write().await.remove(id);

    log::info!("Finished clean-up for {}", id);

    false
}

pub async fn connect(candidate: &CandidateDevice) -> Result<Device, MirajazzError> {
    let firmware_version = Device::read_firmware_version(&candidate.dev).await;

    let firmware_version = match firmware_version {
        Ok(fw) => fw,
        Err(e) => {
            log::error!("Failed to read firmware version from {}", &candidate.id);

            return Err(e);
        }
    };

    log::info!(
        "Connecting to {} with fw {:?}",
        &candidate.id,
        &firmware_version
    );

    let result = Device::connect(
        &candidate.dev,
        candidate.kind.protocol_version(),
        INPUT_KEY_COUNT,
        ENCODER_COUNT,
    )
    .await;

    match result {
        Ok(device) => Ok(device),
        Err(e) => {
            log::error!("Error while connecting to device: {e}");

            Err(e)
        }
    }
}

/// Handles events from device to OpenDeck
async fn device_events_task(candidate: &CandidateDevice) -> Result<(), MirajazzError> {
    log::info!("Connecting to {} for incoming events", candidate.id);

    let devices_lock = DEVICES.read().await;
    let reader = match devices_lock.get(&candidate.id) {
        Some(device) => device.get_reader(crate::inputs::process_input),
        None => return Ok(()),
    };
    drop(devices_lock);

    log::info!("Connected to {} for incoming events", candidate.id);

    log::info!("Reader is ready for {}", candidate.id);

    loop {
        log::info!("Reading updates...");

        let updates = match reader.read(None).await {
            Ok(updates) => updates,
            Err(e) => {
                if !handle_error(&candidate.id, e).await {
                    break;
                }

                continue;
            }
        };

        for update in updates {
            log::info!("New update: {:#?}", update);

            let id = candidate.id.clone();

            if let Some(outbound) = OUTBOUND_EVENT_MANAGER.lock().await.as_mut() {
                match update {
                    DeviceStateUpdate::ButtonDown(key) => outbound.key_down(id, key).await.unwrap(),
                    DeviceStateUpdate::ButtonUp(key) => outbound.key_up(id, key).await.unwrap(),
                    DeviceStateUpdate::EncoderDown(encoder) => {
                        outbound.encoder_down(id, encoder).await.unwrap();
                    }
                    DeviceStateUpdate::EncoderUp(encoder) => {
                        outbound.encoder_up(id, encoder).await.unwrap();
                    }
                    DeviceStateUpdate::EncoderTwist(encoder, val) => {
                        outbound
                            .encoder_change(id, encoder, val as i16)
                            .await
                            .unwrap();
                    }
                }
            }
        }
    }

    Ok(())
}

/// Device writes are kept behind this narrow interface so tests exercise the image consumer
/// without opening a HID handle.
pub(crate) trait ImageOutput {
    async fn write_jpeg(&self, index: u8, jpeg: &[u8]) -> Result<(), MirajazzError>;
    async fn clear_key(&self, index: u8) -> Result<(), MirajazzError>;
    async fn clear_keys(&self) -> Result<(), MirajazzError>;
    async fn flush_images(&self) -> Result<(), MirajazzError>;
}

impl ImageOutput for Device {
    async fn write_jpeg(&self, index: u8, jpeg: &[u8]) -> Result<(), MirajazzError> {
        self.write_image(index, jpeg).await
    }

    async fn clear_key(&self, index: u8) -> Result<(), MirajazzError> {
        self.clear_button_image(index).await
    }

    async fn clear_keys(&self) -> Result<(), MirajazzError> {
        self.clear_all_button_images().await
    }

    async fn flush_images(&self) -> Result<(), MirajazzError> {
        self.flush().await
    }
}

/// Accepts JPEG and lossless PNG frames; each display is encoded only once, by `encode_jpeg`.
pub(crate) fn decode_image(image: &str) -> Result<DynamicImage, MirajazzError> {
    let url = DataUrl::process(image).map_err(|_| MirajazzError::BadData)?;
    let format = match (url.mime_type().type_.as_str(), url.mime_type().subtype.as_str()) {
        ("image", "jpeg") => image::ImageFormat::Jpeg,
        ("image", "png") => image::ImageFormat::Png,
        _ => return Err(MirajazzError::BadData),
    };
    let (body, _) = url.decode_to_vec().map_err(|_| MirajazzError::BadData)?;
    Ok(load_from_memory_with_format(&body, format)?)
}

/// Composites over black (as a canvas JPEG export does), resamples only off-size input,
/// and avoids mirajazz's nearest-neighbour resize and q90 encode.
pub(crate) fn encode_jpeg(image: DynamicImage, size: (u32, u32)) -> Result<Vec<u8>, MirajazzError> {
    let mut pixels = RgbImage::new(image.width(), image.height());
    for (target, source) in pixels.pixels_mut().zip(image.to_rgba8().pixels()) {
        let [r, g, b, a] = source.0;
        target.0 = [r, g, b].map(|channel| ((channel as u16 * a as u16 + 127) / 255) as u8);
    }
    let pixels = if pixels.dimensions() == size {
        pixels
    } else {
        image::imageops::resize(&pixels, size.0, size.1, FilterType::Lanczos3)
    };
    let mut jpeg = Vec::new();
    JpegEncoder::new_with_quality(&mut jpeg, JPEG_QUALITY).encode_image(&pixels)?;
    Ok(jpeg)
}

/// Who is shown on a device's full LCD. A direct-drawing client (see `strip`) owns it until it
/// releases; OpenDeck's Infobar frames are kept meanwhile and shown again afterwards.
#[derive(Default)]
pub struct LcdState {
    /// Connection id of the direct-drawing client, if any.
    pub owner: Option<u64>,
    /// JPEG the owner last drew, redrawn after the device reconnects.
    pub direct: Option<Vec<u8>>,
    /// Latest OpenDeck Infobar frame; `None` is a blank LCD.
    pub opendeck: Option<String>,
}

/// Lock order: take `DEVICES` before `LCD_STATES`, as `set_image` already holds `DEVICES`.
pub static LCD_STATES: LazyLock<Mutex<HashMap<String, LcdState>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

pub(crate) async fn draw_opendeck_frame(output: &impl ImageOutput, image: Option<String>) -> Result<(), MirajazzError> {
    let image = match image {
        Some(image) => decode_image(&image)?,
        // CLE slots do not reliably blank this LCD on the calibrated firmware.
        None => DynamicImage::ImageRgb8(RgbImage::new(LCD_SIZE.0, LCD_SIZE.1)),
    };
    log::info!(
        "event=full_lcd_frame controller=Infobar position=0 input={}x{} output={}x{} mirajazz_index={} bat_wire_slot={}",
        image.width(), image.height(), LCD_SIZE.0, LCD_SIZE.1,
        LCD_IMAGE_INDEX, LCD_IMAGE_INDEX + 1
    );
    write_lcd_jpeg(output, &encode_jpeg(image, LCD_SIZE)?).await
}

pub(crate) async fn write_lcd_jpeg(output: &impl ImageOutput, jpeg: &[u8]) -> Result<(), MirajazzError> {
    output.write_jpeg(LCD_IMAGE_INDEX, jpeg).await?;
    output.flush_images().await
}

/// Records OpenDeck's latest LCD frame and shows it unless a direct client owns the LCD.
async fn draw_lcd(output: &impl ImageOutput, lcd: &mut LcdState, image: Option<String>) -> Result<(), MirajazzError> {
    lcd.opendeck = image.clone();
    if lcd.owner.is_some() {
        return Ok(());
    }
    draw_opendeck_frame(output, image).await
}

async fn apply_image(output: &impl ImageOutput, lcd: &mut LcdState, evt: SetImageEvent) -> Result<(), MirajazzError> {
    match (evt.controller.as_deref(), evt.position) {
        (Some("Encoder"), Some(0) | None) => Ok(()),
        (Some("Infobar"), Some(0)) => draw_lcd(output, lcd, evt.image).await,
        (Some("Infobar"), None) if evt.image.is_none() => draw_lcd(output, lcd, None).await,
        (Some("Keypad") | None, Some(position)) if (position as usize) < INPUT_KEY_COUNT => {
            if position as usize >= KEY_COUNT {
                return Ok(()); // Touch points have no display, even though their indices overlap BAT slots.
            }
            match evt.image {
                Some(image) => {
                    let image = decode_image(&image)?;
                    log::info!("event=key_frame position={position} input={}x{} output={}x{}", image.width(), image.height(), KEY_SIZE.0, KEY_SIZE.1);
                    output.write_jpeg(position, &encode_jpeg(image, KEY_SIZE)?).await?
                }
                None => output.clear_key(position).await?,
            }
            output.flush_images().await
        }
        (Some("Keypad") | None, None) if evt.image.is_none() => {
            output.clear_keys().await?;
            draw_lcd(output, lcd, None).await
        }
        _ => Err(MirajazzError::BadData),
    }
}

/// Only Infobar 0 owns the full LCD. Input-only controllers never draw into its BAT slot.
pub async fn handle_set_image(device: &Device, evt: SetImageEvent) -> Result<(), MirajazzError> {
    let mut states = LCD_STATES.lock().await;
    let lcd = states.entry(evt.device.clone()).or_default();
    apply_image(device, lcd, evt).await
}

/// Redraws a direct client's last frame after the device (re)connects; OpenDeck repaints its own.
async fn restore_direct_lcd(id: &str) {
    let devices = DEVICES.read().await;
    let states = LCD_STATES.lock().await;
    if let (Some(device), Some(jpeg)) = (devices.get(id), states.get(id).and_then(|lcd| lcd.direct.as_ref())) {
        if let Err(err) = write_lcd_jpeg(device, jpeg).await {
            log::error!("Failed to restore direct LCD frame on {id}: {err}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, fmt::Write, io::Cursor};

    #[derive(Debug, PartialEq)]
    enum WriteOperation {
        Draw(u8, u32, u32, [u8; 3]),
        Clear(u8),
        ClearAll,
        Flush,
    }

    #[derive(Default)]
    struct RecordingOutput(RefCell<Vec<WriteOperation>>, RefCell<Option<RgbImage>>);

    impl RecordingOutput {
        fn record(&self, index: u8, jpeg: &[u8]) -> Result<(), MirajazzError> {
            let jpeg = load_from_memory_with_format(jpeg, image::ImageFormat::Jpeg)?.to_rgb8();
            self.0.borrow_mut().push(WriteOperation::Draw(index, jpeg.width(), jpeg.height(), jpeg.get_pixel(0, 0).0));
            *self.1.borrow_mut() = Some(jpeg);
            Ok(())
        }
    }

    impl ImageOutput for RecordingOutput {
        async fn write_jpeg(&self, index: u8, jpeg: &[u8]) -> Result<(), MirajazzError> {
            self.record(index, jpeg)
        }

        async fn clear_key(&self, index: u8) -> Result<(), MirajazzError> {
            self.0.borrow_mut().push(WriteOperation::Clear(index));
            Ok(())
        }

        async fn clear_keys(&self) -> Result<(), MirajazzError> {
            self.0.borrow_mut().push(WriteOperation::ClearAll);
            Ok(())
        }

        async fn flush_images(&self) -> Result<(), MirajazzError> {
            self.0.borrow_mut().push(WriteOperation::Flush);
            Ok(())
        }
    }

    fn event(controller: Option<&str>, position: Option<u8>, image: Option<String>) -> SetImageEvent {
        SetImageEvent { device: "n1-test".into(), controller: controller.map(str::to_owned), position, image }
    }

    /// Applies an event with no direct client owning the LCD.
    async fn apply(output: &RecordingOutput, evt: SetImageEvent) -> Result<(), MirajazzError> {
        apply_image(output, &mut LcdState::default(), evt).await
    }

    fn black_jpeg_url() -> String {
        let mut jpeg = Cursor::new(Vec::new());
        DynamicImage::ImageRgb8(RgbImage::new(12, 12)).write_to(&mut jpeg, image::ImageFormat::Jpeg).unwrap();
        let mut url = String::from("data:image/jpeg,");
        for byte in jpeg.into_inner() {
            write!(&mut url, "%{byte:02X}").unwrap();
        }
        url
    }

    #[tokio::test]
    async fn native_png_frames_keep_single_pixel_detail_and_flatten_alpha_to_black() {
        for (controller, position, size) in [("Infobar", 0, LCD_SIZE), ("Keypad", 14, KEY_SIZE)] {
            let (line, transparent, row) = (size.0 / 2, size.0 / 2 + 20, size.1 / 2);
            let mut frame = image::RgbaImage::from_pixel(size.0, size.1, image::Rgba([0, 0, 0, 255]));
            for y in 0..size.1 {
                frame.put_pixel(line, y, image::Rgba([255, 255, 255, 255]));
                frame.put_pixel(transparent, y, image::Rgba([255, 255, 255, 0]));
            }
            let mut png = Cursor::new(Vec::new());
            frame.write_to(&mut png, image::ImageFormat::Png).unwrap();
            let url = format!("data:image/png,{}", png.into_inner().iter().map(|byte| format!("%{byte:02X}")).collect::<String>());

            let output = RecordingOutput::default();
            apply(&output, event(Some(controller), Some(position), Some(url))).await.unwrap();

            let written = output.1.borrow().clone().unwrap();
            assert_eq!(written.dimensions(), size, "{controller}");
            let luma = |x: u32| written.get_pixel(x, row).0.iter().map(|&c| c as u32).sum::<u32>() / 3;
            assert!(luma(line) > 230, "{controller}: 1px line lost intensity: {}", luma(line));
            assert!(luma(line - 2) < 25 && luma(line + 2) < 25, "{controller}: 1px line smeared: {} {}", luma(line - 2), luma(line + 2));
            assert!(luma(transparent) < 25, "{controller}: transparent pixel not flattened to black: {}", luma(transparent));
        }
    }

    #[tokio::test]
    async fn only_infobar_image_writes_full_lcd_without_input_controller_collisions() {
        let output = RecordingOutput::default();
        apply(&output, event(Some("Infobar"), Some(0), Some(black_jpeg_url()))).await.unwrap();
        for (controller, position) in [("Keypad", 15), ("Keypad", 16), ("Encoder", 0)] {
            for image in [Some("not a data URL".into()), None] {
                apply(&output, event(Some(controller), Some(position), image)).await.unwrap();
            }
        }
        assert_eq!(*output.0.borrow(), [WriteOperation::Draw(15, 450, 85, [0, 0, 0]), WriteOperation::Flush]);
    }

    #[tokio::test]
    async fn infobar_clear_draws_and_flushes_black_jpeg_instead_of_cle() {
        let output = RecordingOutput::default();
        apply(&output, event(Some("Infobar"), Some(0), None)).await.unwrap();
        assert_eq!(*output.0.borrow(), [WriteOperation::Draw(15, 450, 85, [0, 0, 0]), WriteOperation::Flush]);
    }

    #[tokio::test]
    async fn whole_device_clear_also_blanks_full_lcd() {
        let output = RecordingOutput::default();
        apply(&output, event(None, None, None)).await.unwrap();
        assert_eq!(*output.0.borrow(), [WriteOperation::ClearAll, WriteOperation::Draw(15, 450, 85, [0, 0, 0]), WriteOperation::Flush]);
    }

    #[tokio::test]
    async fn keypad_images_keep_their_existing_geometry_and_clear_slot() {
        let output = RecordingOutput::default();
        apply(&output, event(Some("Keypad"), Some(14), Some(black_jpeg_url()))).await.unwrap();
        apply(&output, event(Some("Keypad"), Some(14), None)).await.unwrap();
        assert_eq!(*output.0.borrow(), [WriteOperation::Draw(14, 105, 100, [0, 0, 0]), WriteOperation::Flush, WriteOperation::Clear(14), WriteOperation::Flush]);
    }

    #[tokio::test]
    async fn directly_owned_lcd_hides_opendeck_frames_but_keeps_the_latest_and_keys_draw() {
        let output = RecordingOutput::default();
        let mut lcd = LcdState { owner: Some(7), ..Default::default() };
        apply_image(&output, &mut lcd, event(Some("Infobar"), Some(0), Some(black_jpeg_url()))).await.unwrap();
        apply_image(&output, &mut lcd, event(None, None, None)).await.unwrap();
        apply_image(&output, &mut lcd, event(Some("Keypad"), Some(3), Some(black_jpeg_url()))).await.unwrap();
        assert_eq!(*output.0.borrow(), [WriteOperation::ClearAll, WriteOperation::Draw(3, 105, 100, [0, 0, 0]), WriteOperation::Flush]);
        assert_eq!(lcd.opendeck, None, "a whole-device clear is the latest OpenDeck LCD frame");

        apply_image(&output, &mut lcd, event(Some("Infobar"), Some(0), Some(black_jpeg_url()))).await.unwrap();
        assert_eq!(lcd.opendeck, Some(black_jpeg_url()));
        assert_eq!(output.0.borrow().len(), 3, "owned LCD must not be drawn");
    }

    #[tokio::test]
    async fn invalid_positions_and_malformed_images_fail_without_drawing() {
        let output = RecordingOutput::default();
        for (controller, position) in [("Infobar", Some(1)), ("Infobar", None), ("Keypad", Some(17)), ("Encoder", Some(1)), ("Unknown", Some(0))] {
            assert!(matches!(apply(&output, event(Some(controller), position, Some(black_jpeg_url()))).await, Err(MirajazzError::BadData)));
        }
        for image in ["not a data URL", "data:image/jpeg;base64,%%%", "data:text/jpeg,test"] {
            assert!(matches!(apply(&output, event(Some("Infobar"), Some(0), Some(image.into()))).await, Err(MirajazzError::BadData)));
        }
        assert!(output.0.borrow().is_empty());
    }
}
