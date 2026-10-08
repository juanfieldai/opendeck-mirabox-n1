//! Direct full-LCD drawing for other plugins, bypassing OpenDeck's 248x58 Infobar raster.
//!
//! Protocol: newline-delimited JSON over the Unix socket
//! `$XDG_RUNTIME_DIR/opendeck-mirabox-n1/strip.sock` (its directory is mode 0700).
//!
//! - `{"event":"drawStrip","device":"n1-…","image":"<data URL>"}` with an `image/svg+xml`,
//!   `image/png` or `image/jpeg` data URL. SVG is rendered to fill the 450x85 LCD; bitmaps of
//!   another size are resampled. The connection then owns that device's LCD: OpenDeck's
//!   Infobar frames are kept but not shown.
//! - `{"event":"releaseStrip","device":"n1-…"}` shows OpenDeck's latest Infobar frame again.
//!
//! Each request gets one reply line, `{"ok":true}` or `{"ok":false,"error":"…"}`. Closing the
//! connection releases every LCD it owns. While one connection owns a device's LCD, `drawStrip`
//! from another connection is rejected.

use std::{
    collections::HashSet,
    fs::Permissions,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock},
};

use data_url::DataUrl;
use futures_lite::StreamExt;
use image::{DynamicImage, RgbaImage};
use resvg::{tiny_skia, usvg};
use serde_json::{Value, json};
use tokio::{
    io::AsyncWriteExt,
    net::{UnixListener, UnixStream},
};
use tokio_util::{
    codec::{FramedRead, LinesCodec},
    sync::CancellationToken,
};

use crate::{
    DEVICES,
    device::{LCD_STATES, decode_image, draw_opendeck_frame, encode_jpeg, write_lcd_jpeg},
    mappings::LCD_SIZE,
};

/// Largest accepted request line; a 450x85 frame as base64 PNG or as SVG is far below this.
const MAX_REQUEST_BYTES: usize = 4 * 1024 * 1024;

static FONTS: LazyLock<Arc<usvg::fontdb::Database>> = LazyLock::new(|| {
    let mut fonts = usvg::fontdb::Database::new();
    fonts.load_system_fonts();
    Arc::new(fonts)
});

fn socket_path() -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR").map(|dir| PathBuf::from(dir).join("opendeck-mirabox-n1").join("strip.sock"))
}

fn bind(path: &Path) -> std::io::Result<UnixListener> {
    let dir = path.parent().expect("socket path has a parent directory");
    std::fs::create_dir_all(dir)?;
    std::fs::set_permissions(dir, Permissions::from_mode(0o700))?;
    match std::fs::remove_file(path) {
        Err(err) if err.kind() != std::io::ErrorKind::NotFound => return Err(err),
        _ => {}
    }
    UnixListener::bind(path)
}

pub async fn serve(token: CancellationToken) {
    let Some(path) = socket_path() else {
        log::warn!("XDG_RUNTIME_DIR is unset; direct strip drawing is disabled");
        return;
    };
    let listener = match bind(&path) {
        Ok(listener) => listener,
        Err(err) => {
            log::error!("Failed to listen on {}: {err}", path.display());
            return;
        }
    };
    log::info!("event=strip_socket_listening path={}", path.display());

    let mut next_connection = 1;
    loop {
        tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => {
                    tokio::spawn(serve_connection(next_connection, stream));
                    next_connection += 1;
                }
                Err(err) => log::error!("Strip socket accept failed: {err}"),
            },
            _ = token.cancelled() => break,
        }
    }
    let _ = std::fs::remove_file(&path);
}

async fn serve_connection(connection: u64, stream: UnixStream) {
    let (read, mut write) = stream.into_split();
    let mut requests = FramedRead::new(read, LinesCodec::new_with_max_length(MAX_REQUEST_BYTES));
    let mut owned = HashSet::new();

    while let Some(line) = requests.next().await {
        let (reply, fatal) = match line {
            Ok(line) => match handle_request(connection, &line, &mut owned).await {
                Ok(()) => (json!({ "ok": true }), false),
                Err(error) => (json!({ "ok": false, "error": error }), false),
            },
            // An oversized or broken line cannot be resynchronised.
            Err(error) => (json!({ "ok": false, "error": error.to_string() }), true),
        };
        if write.write_all(format!("{reply}\n").as_bytes()).await.is_err() || fatal {
            break;
        }
    }

    for device in owned {
        release(connection, &device).await;
    }
}

async fn handle_request(connection: u64, line: &str, owned: &mut HashSet<String>) -> Result<(), String> {
    let request: Value = serde_json::from_str(line).map_err(|err| format!("invalid JSON: {err}"))?;
    let device = request["device"].as_str().ok_or("missing device")?;
    match request["event"].as_str() {
        Some("drawStrip") => {
            let image = request["image"].as_str().ok_or("missing image")?;
            let jpeg = encode_jpeg(render(image)?, LCD_SIZE).map_err(|err| err.to_string())?;
            draw(connection, device, jpeg).await?;
            owned.insert(device.to_owned());
            Ok(())
        }
        Some("releaseStrip") => {
            release(connection, device).await;
            owned.remove(device);
            Ok(())
        }
        _ => Err("unknown event".to_owned()),
    }
}

/// Decodes a data URL; SVG is rendered directly at the LCD's native size.
fn render(image: &str) -> Result<DynamicImage, String> {
    let url = DataUrl::process(image).map_err(|_| "image must be a data URL")?;
    if url.mime_type().type_ != "image" || url.mime_type().subtype != "svg+xml" {
        return decode_image(image).map_err(|_| "image must be an SVG, PNG or JPEG data URL".to_owned());
    }
    let (svg, _) = url.decode_to_vec().map_err(|_| "invalid data URL body")?;
    let options = usvg::Options { fontdb: FONTS.clone(), ..Default::default() };
    let tree = usvg::Tree::from_data(&svg, &options).map_err(|err| format!("invalid SVG: {err}"))?;

    let (width, height) = LCD_SIZE;
    let mut pixmap = tiny_skia::Pixmap::new(width, height).ok_or("cannot allocate LCD frame")?;
    let scale = tiny_skia::Transform::from_scale(width as f32 / tree.size().width(), height as f32 / tree.size().height());
    resvg::render(&tree, scale, &mut pixmap.as_mut());
    let pixels = RgbaImage::from_raw(width, height, pixmap.take_demultiplied()).ok_or("cannot read LCD frame")?;
    Ok(DynamicImage::ImageRgba8(pixels))
}

async fn draw(connection: u64, id: &str, jpeg: Vec<u8>) -> Result<(), String> {
    let devices = DEVICES.read().await;
    let device = devices.get(id).ok_or("device not connected")?;
    let mut states = LCD_STATES.lock().await;
    let lcd = states.entry(id.to_owned()).or_default();
    if lcd.owner.is_some_and(|owner| owner != connection) {
        return Err("LCD is owned by another client".to_owned());
    }
    write_lcd_jpeg(device, &jpeg).await.map_err(|err| err.to_string())?;
    lcd.owner = Some(connection);
    lcd.direct = Some(jpeg);
    log::info!("event=direct_strip_frame device={id} connection={connection} output={}x{}", LCD_SIZE.0, LCD_SIZE.1);
    Ok(())
}

async fn release(connection: u64, id: &str) {
    let devices = DEVICES.read().await;
    let mut states = LCD_STATES.lock().await;
    let Some(lcd) = states.get_mut(id).filter(|lcd| lcd.owner == Some(connection)) else {
        return;
    };
    lcd.owner = None;
    lcd.direct = None;
    log::info!("event=direct_strip_released device={id} connection={connection}");
    if let Some(device) = devices.get(id)
        && let Err(err) = draw_opendeck_frame(device, lcd.opendeck.clone()).await
    {
        log::error!("Failed to restore OpenDeck LCD frame on {id}: {err}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt, BufReader};

    fn svg_url(svg: &str) -> String {
        format!("data:image/svg+xml,{}", svg.bytes().map(|byte| format!("%{byte:02X}")).collect::<String>())
    }

    #[test]
    fn native_svg_keeps_single_pixel_lines_and_other_sizes_fill_the_lcd() {
        let line = svg_url(r##"<svg xmlns="http://www.w3.org/2000/svg" width="450" height="85"><rect width="450" height="85"/><rect x="200" width="1" height="85" fill="#fff"/></svg>"##);
        let frame = render(&line).unwrap().to_rgba8();
        assert_eq!(frame.dimensions(), LCD_SIZE);
        assert_eq!(frame.get_pixel(200, 42).0, [255, 255, 255, 255]);
        assert_eq!(frame.get_pixel(199, 42).0, [0, 0, 0, 255]);
        assert_eq!(frame.get_pixel(201, 42).0, [0, 0, 0, 255]);

        let small = svg_url(r##"<svg xmlns="http://www.w3.org/2000/svg" width="90" height="17"><rect width="90" height="17" fill="#f00"/></svg>"##);
        let frame = render(&small).unwrap().to_rgba8();
        assert_eq!(frame.dimensions(), LCD_SIZE);
        assert_eq!(frame.get_pixel(449, 84).0, [255, 0, 0, 255]);
    }

    #[tokio::test]
    async fn every_request_gets_a_reply_and_failures_change_nothing() {
        let (client, server) = UnixStream::pair().unwrap();
        tokio::spawn(serve_connection(1, server));
        let (read, mut write) = client.into_split();
        let mut replies = BufReader::new(read).lines();

        for (request, expected) in [
            ("not json", "invalid JSON"),
            (r#"{"event":"drawStrip"}"#, "missing device"),
            (r#"{"event":"scroll","device":"n1-test"}"#, "unknown event"),
            (r#"{"event":"drawStrip","device":"n1-test","image":"data:text/plain,hi"}"#, "image must be an SVG, PNG or JPEG data URL"),
            (r#"{"event":"drawStrip","device":"n1-absent","image":"data:image/svg+xml,%3Csvg%20xmlns%3D%22http%3A%2F%2Fwww.w3.org%2F2000%2Fsvg%22%20width%3D%221%22%20height%3D%221%22%2F%3E"}"#, "device not connected"),
        ] {
            write.write_all(format!("{request}\n").as_bytes()).await.unwrap();
            let reply: Value = serde_json::from_str(&replies.next_line().await.unwrap().unwrap()).unwrap();
            assert_eq!(reply["ok"], false, "{request}");
            assert!(reply["error"].as_str().unwrap().starts_with(expected), "{request}: {reply}");
        }

        write.write_all(b"{\"event\":\"releaseStrip\",\"device\":\"n1-absent\"}\n").await.unwrap();
        assert_eq!(replies.next_line().await.unwrap().unwrap(), r#"{"ok":true}"#);
        assert!(LCD_STATES.lock().await.get("n1-absent").is_none_or(|lcd| lcd.owner.is_none()));
    }
}
