use std::collections::HashMap;
use std::io::Read;

use tiny_skia::{IntSize, Pixmap};
use zbus::blocking::Connection;
use zbus::zvariant::{Fd, OwnedValue, Value};

const FORMAT_RGB32: u32 = 4;
const FORMAT_ARGB32: u32 = 5;
const FORMAT_ARGB32_PREMULTIPLIED: u32 = 6;
const FORMAT_RGBX8888: u32 = 16;
const FORMAT_RGBA8888: u32 = 17;
const FORMAT_RGBA8888_PREMULTIPLIED: u32 = 18;

pub fn capture_active_screen(conn: &Connection) -> Result<Pixmap, String> {
    let (mut read_end, write_end) = std::io::pipe().map_err(|e| format!("pipe: {e}"))?;

    let reader = std::thread::spawn(move || {
        let mut data = Vec::with_capacity(1920 * 1080 * 4);
        read_end.read_to_end(&mut data).map(|_| data)
    });

    let mut options: HashMap<&str, Value> = HashMap::new();
    options.insert("include-cursor", Value::from(false));
    options.insert("native-resolution", Value::from(true));

    let reply = conn.call_method(
        Some("org.kde.KWin"),
        "/org/kde/KWin/ScreenShot2",
        Some("org.kde.KWin.ScreenShot2"),
        "CaptureActiveScreen",
        &(options, Fd::from(&write_end)),
    );
    drop(write_end);

    let reply = reply.map_err(|e| format!("KWin ScreenShot2: {e}"))?;
    let meta: HashMap<String, OwnedValue> = reply.body().deserialize().map_err(|e| format!("KWin reply: {e}"))?;

    let data = reader
        .join()
        .map_err(|_| "reader thread panicked".to_string())?
        .map_err(|e| format!("pipe read: {e}"))?;

    let get = |key: &str| -> Result<u32, String> {
        meta.get(key)
            .and_then(|v| u32::try_from(v).ok())
            .ok_or_else(|| format!("KWin reply missing {key}"))
    };
    let width = get("width")?;
    let height = get("height")?;
    let stride = get("stride")? as usize;
    let format = get("format")?;

    to_pixmap(&data, width, height, stride, format)
}

fn to_pixmap(data: &[u8], width: u32, height: u32, stride: usize, format: u32) -> Result<Pixmap, String> {
    let row_bytes = width as usize * 4;
    if stride < row_bytes || data.len() < stride * (height as usize - 1) + row_bytes {
        return Err(format!(
            "short image data: {} bytes for {width}x{height} stride {stride}",
            data.len()
        ));
    }

    let bgra = match format {
        FORMAT_RGB32 | FORMAT_ARGB32 | FORMAT_ARGB32_PREMULTIPLIED => true,
        FORMAT_RGBX8888 | FORMAT_RGBA8888 | FORMAT_RGBA8888_PREMULTIPLIED => false,
        other => return Err(format!("unsupported image format {other}")),
    };

    let mut out = Vec::with_capacity(row_bytes * height as usize);
    for row in data.chunks(stride).take(height as usize) {
        for px in row[..row_bytes].as_chunks::<4>().0 {
            if bgra {
                out.extend_from_slice(&[px[2], px[1], px[0], 255]);
            } else {
                out.extend_from_slice(&[px[0], px[1], px[2], 255]);
            }
        }
    }

    let size = IntSize::from_wh(width, height).ok_or("empty screenshot")?;
    Pixmap::from_vec(out, size).ok_or_else(|| "invalid pixmap".to_string())
}
