use std::collections::HashMap;
use std::io::Read;

use tiny_skia::{IntSize, Pixmap};
use zbus::{Connection, Fd, OwnedVariant, Variant};

use crate::draw;

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

    let mut options: HashMap<&str, Variant> = HashMap::new();
    options.insert("include-cursor", Variant::from(true));
    options.insert("native-resolution", Variant::from(true));

    let reply = zbus::block_on(conn.call_method(
        Some("org.kde.KWin"),
        "/org/kde/KWin/ScreenShot2",
        Some("org.kde.KWin.ScreenShot2"),
        "CaptureActiveScreen",
        &(options, Fd::from(&write_end)),
    ));
    drop(write_end);

    let reply = reply.map_err(|e| format!("KWin ScreenShot2: {e}"))?;
    let meta: HashMap<String, OwnedVariant> = reply.body().deserialize().map_err(|e| format!("KWin reply: {e}"))?;

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

    to_pixmap(data, width, height, stride, format)
}

fn to_pixmap(mut data: Vec<u8>, width: u32, height: u32, stride: usize, format: u32) -> Result<Pixmap, String> {
    let row_bytes = width as usize * 4;
    let rows = height as usize;
    if rows == 0 || stride < row_bytes || data.len() < stride * (rows - 1) + row_bytes {
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

    if stride != row_bytes {
        for row in 1..rows {
            data.copy_within(row * stride..row * stride + row_bytes, row * row_bytes);
        }
    }
    data.truncate(row_bytes * rows);
    if !bgra {
        draw::swap_rb(&mut data);
    }
    draw::make_opaque(&mut data);

    let size = IntSize::from_wh(width, height).ok_or("empty screenshot")?;
    Pixmap::from_vec(data, size).ok_or_else(|| "invalid pixmap".to_string())
}
