use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::OnceLock;

use fontdue::{Font, FontSettings, Metrics};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Weight {
    Regular,
    Medium,
}

struct Fonts {
    regular: Font,
    medium: Font,
}

static FONTS: OnceLock<Option<Fonts>> = OnceLock::new();

type Glyph = (Metrics, Vec<u8>);

thread_local! {
    static GLYPHS: RefCell<HashMap<(Weight, char, u32), Glyph>> = RefCell::new(HashMap::new());
}

pub fn warm() {
    std::thread::spawn(|| {
        fonts();
    });
}

fn fonts() -> Option<&'static Fonts> {
    FONTS
        .get_or_init(|| {
            let regular = load("sans-serif:style=Regular", "NotoSans-Regular.ttf")?;
            let medium = load("sans-serif:style=Medium", "NotoSans-Medium.ttf")
                .or_else(|| load("sans-serif:style=Bold", "NotoSans-Bold.ttf"))?;
            Some(Fonts { regular, medium })
        })
        .as_ref()
}

fn load(pattern: &str, fallback: &str) -> Option<Font> {
    let matched = std::process::Command::new("fc-match")
        .args(["-f", "%{file}", pattern])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .filter(|p| !p.is_empty());
    let candidates = matched.into_iter().chain(
        [
            "/usr/share/fonts/noto/",
            "/usr/share/fonts/truetype/noto/",
            "/usr/share/fonts/google-noto/",
        ]
        .iter()
        .map(|d| format!("{d}{fallback}")),
    );
    for path in candidates {
        if let Ok(bytes) = std::fs::read(&path) {
            match Font::from_bytes(bytes, FontSettings::default()) {
                Ok(font) => return Some(font),
                Err(e) => eprintln!("ahri: font {path}: {e}"),
            }
        }
    }
    eprintln!("ahri: no font for {pattern}");
    None
}

fn font(weight: Weight) -> Option<&'static Font> {
    let f = fonts()?;
    Some(match weight {
        Weight::Regular => &f.regular,
        Weight::Medium => &f.medium,
    })
}

fn with_glyph<R>(weight: Weight, ch: char, px: u32, f: impl FnOnce(&Glyph) -> R) -> Option<R> {
    let font = font(weight)?;
    GLYPHS.with_borrow_mut(|cache| {
        let glyph = cache
            .entry((weight, ch, px))
            .or_insert_with(|| font.rasterize(ch, px as f32));
        Some(f(glyph))
    })
}

pub fn width(text: &str, size: f32, weight: Weight) -> f32 {
    let px = size.round().max(1.0) as u32;
    let Some(font) = font(weight) else { return 0.0 };
    let mut w = 0.0;
    let mut prev = None;
    for ch in text.chars() {
        if let Some(p) = prev {
            w += font.horizontal_kern(p, ch, px as f32).unwrap_or(0.0);
        }
        w += with_glyph(weight, ch, px, |(m, _)| m.advance_width).unwrap_or(0.0);
        prev = Some(ch);
    }
    w
}

pub struct Canvas<'a> {
    pub data: &'a mut [u8],
    pub width: u32,
    pub height: u32,
}

pub fn draw(canvas: &mut Canvas, x: f32, baseline: f32, size: f32, weight: Weight, rgba: [u8; 4], text: &str) -> f32 {
    let px = size.round().max(1.0) as u32;
    let Some(font) = font(weight) else { return x };
    let mut pen = x;
    let mut prev = None;
    for ch in text.chars() {
        if let Some(p) = prev {
            pen += font.horizontal_kern(p, ch, px as f32).unwrap_or(0.0);
        }
        let advance = with_glyph(weight, ch, px, |(m, coverage)| {
            let gx = (pen + m.xmin as f32).round() as i32;
            let gy = (baseline - m.height as f32 - m.ymin as f32).round() as i32;
            blit(canvas, gx, gy, m.width, m.height, coverage, rgba);
            m.advance_width
        })
        .unwrap_or(0.0);
        pen += advance;
        prev = Some(ch);
    }
    pen
}

fn blit(canvas: &mut Canvas, gx: i32, gy: i32, gw: usize, gh: usize, coverage: &[u8], [r, g, b, a]: [u8; 4]) {
    let (cw, ch) = (canvas.width as i32, canvas.height as i32);
    let x0 = gx.max(0);
    let x1 = (gx + gw as i32).min(cw);
    if x0 >= x1 {
        return;
    }
    for row in 0..gh as i32 {
        let y = gy + row;
        if y < 0 || y >= ch {
            continue;
        }
        let src = &coverage[row as usize * gw..];
        let line = &mut canvas.data[(y * cw) as usize * 4..];
        for x in x0..x1 {
            let c = src[(x - gx) as usize] as u32;
            if c == 0 {
                continue;
            }
            let k = c * a as u32 / 255;
            let inv = 255 - k;
            let o = x as usize * 4;
            let d = &mut line[o..o + 4];
            d[0] = ((b as u32 * k + d[0] as u32 * inv) / 255) as u8;
            d[1] = ((g as u32 * k + d[1] as u32 * inv) / 255) as u8;
            d[2] = ((r as u32 * k + d[2] as u32 * inv) / 255) as u8;
            d[3] = (k + d[3] as u32 * inv / 255) as u8;
        }
    }
}
