use tiny_skia::{
    Color, FillRule, LineCap, LineJoin, Mask, Paint, PathBuilder, Pixmap, PixmapPaint, Point, Rect, Stroke, Transform,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Draw,
    Arrow,
}

#[derive(Debug, Clone)]
pub enum Mark {
    Draw(Vec<Point>, Color),
    Arrow(Point, Point, Color),
}

pub const PALETTE: [(u8, u8, u8); 6] = [
    (235, 40, 40),
    (250, 204, 21),
    (34, 197, 94),
    (59, 130, 246),
    (255, 255, 255),
    (17, 17, 17),
];

pub fn palette(i: usize) -> Color {
    let (r, g, b) = PALETTE[i % PALETTE.len()];
    Color::from_rgba8(r, g, b, 255)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sel {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Sel {
    pub fn from_points(a: Point, b: Point) -> Self {
        Self {
            x: a.x.min(b.x),
            y: a.y.min(b.y),
            w: (a.x - b.x).abs(),
            h: (a.y - b.y).abs(),
        }
    }

    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.x && p.x <= self.x + self.w && p.y >= self.y && p.y <= self.y + self.h
    }

    pub fn rect(&self) -> Option<Rect> {
        Rect::from_xywh(self.x, self.y, self.w, self.h)
    }

    pub fn snapped(&self, max_w: u32, max_h: u32) -> (u32, u32, u32, u32) {
        let x0 = self.x.round().clamp(0.0, max_w as f32) as u32;
        let y0 = self.y.round().clamp(0.0, max_h as f32) as u32;
        let x1 = (self.x + self.w).round().clamp(0.0, max_w as f32) as u32;
        let y1 = (self.y + self.h).round().clamp(0.0, max_h as f32) as u32;
        (x0, y0, x1.saturating_sub(x0), y1.saturating_sub(y0))
    }
}

pub fn frame_white() -> Color {
    Color::WHITE
}

pub fn frame_red() -> Color {
    Color::from_rgba8(255, 80, 80, 255)
}

pub fn paint(color: Color) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color(color);
    p.anti_alias = true;
    p
}

pub fn draw_mark(canvas: &mut Pixmap, mark: &Mark, width: f32, transform: Transform, mask: Option<&Mask>) {
    let stroke = Stroke {
        width,
        line_cap: LineCap::Round,
        line_join: LineJoin::Round,
        ..Stroke::default()
    };
    let color = match mark {
        Mark::Draw(_, c) | Mark::Arrow(_, _, c) => *c,
    };
    let p = paint(color);
    match mark {
        Mark::Draw(points, _) => {
            let Some(first) = points.first() else { return };
            if points.len() == 1 {
                let r = width / 2.0;
                if let Some(path) = PathBuilder::from_circle(first.x, first.y, r) {
                    canvas.fill_path(&path, &p, FillRule::Winding, transform, mask);
                }
                return;
            }
            let mut pb = PathBuilder::new();
            pb.move_to(first.x, first.y);
            for pt in &points[1..] {
                pb.line_to(pt.x, pt.y);
            }
            if let Some(path) = pb.finish() {
                canvas.stroke_path(&path, &p, &stroke, transform, mask);
            }
        }
        Mark::Arrow(a, b, _) => {
            let (dx, dy) = (b.x - a.x, b.y - a.y);
            let len = (dx * dx + dy * dy).sqrt();
            if len < 1.0 {
                return;
            }
            let (ux, uy) = (dx / len, dy / len);
            let head_len = (width * 4.5).min(len);
            let head_half = width * 2.5;
            let base = Point::from_xy(b.x - ux * head_len, b.y - uy * head_len);

            let mut shaft = PathBuilder::new();
            shaft.move_to(a.x, a.y);
            shaft.line_to(base.x + ux * width * 0.5, base.y + uy * width * 0.5);
            if let Some(path) = shaft.finish() {
                canvas.stroke_path(&path, &p, &stroke, transform, mask);
            }

            let mut head = PathBuilder::new();
            head.move_to(b.x, b.y);
            head.line_to(base.x - uy * head_half, base.y + ux * head_half);
            head.line_to(base.x + uy * head_half, base.y - ux * head_half);
            head.close();
            if let Some(path) = head.finish() {
                let head_stroke = Stroke {
                    width: width * 0.5,
                    line_join: LineJoin::Round,
                    ..Stroke::default()
                };
                canvas.fill_path(&path, &p, FillRule::Winding, transform, mask);
                canvas.stroke_path(&path, &p, &head_stroke, transform, mask);
            }
        }
    }
}

pub fn dimmed(src: &Pixmap, factor: f32) -> Pixmap {
    let mut out = src.clone();
    let f = (factor * 256.0) as u16;
    for px in out.data_mut().as_chunks_mut::<4>().0 {
        px[0] = ((px[0] as u16 * f) >> 8) as u8;
        px[1] = ((px[1] as u16 * f) >> 8) as u8;
        px[2] = ((px[2] as u16 * f) >> 8) as u8;
    }
    out
}

pub fn copy_rect(dst: &mut Pixmap, src: &Pixmap, x: u32, y: u32, w: u32, h: u32) {
    let stride = src.width() as usize * 4;
    let (start, len) = (x as usize * 4, w as usize * 4);
    let src_data = src.data();
    let dst_data = dst.data_mut();
    for row in y as usize..(y + h) as usize {
        let off = row * stride + start;
        dst_data[off..off + len].copy_from_slice(&src_data[off..off + len]);
    }
}

pub fn crop(src: &Pixmap, x: u32, y: u32, w: u32, h: u32) -> Option<Pixmap> {
    let mut out = Pixmap::new(w, h)?;
    out.draw_pixmap(
        -(x as i32),
        -(y as i32),
        src.as_ref(),
        &PixmapPaint::default(),
        Transform::identity(),
        None,
    );
    Some(out)
}

pub fn selection_frame(canvas: &mut Pixmap, sel: &Sel, gap: f32, color: Color, scale: f32) {
    let line_w = 1.0 * scale;
    let corner_w = 3.0 * scale;
    let len = 14.0 * scale;
    let radius = 4.0 * scale;
    let g = gap * scale;

    let mut line_color = color;
    line_color.set_alpha(0.4);
    let o = g + line_w / 2.0;
    if let Some(rect) = Rect::from_xywh(sel.x - o, sel.y - o, sel.w + o * 2.0, sel.h + o * 2.0) {
        let stroke = Stroke {
            width: line_w,
            ..Stroke::default()
        };
        canvas.stroke_path(
            &PathBuilder::from_rect(rect),
            &paint(line_color),
            &stroke,
            Transform::identity(),
            None,
        );
    }

    let o = g + corner_w / 2.0;
    let (x0, y0, x1, y1) = (sel.x - o, sel.y - o, sel.x + sel.w + o, sel.y + sel.h + o);
    let len = len.min((x1 - x0) / 2.0).min((y1 - y0) / 2.0);
    let r = radius.min(len);
    let k = r * 0.5523;
    let mut pb = PathBuilder::new();
    for (cx, cy, sx, sy) in [
        (x0, y0, 1.0, 1.0),
        (x1, y0, -1.0, 1.0),
        (x0, y1, 1.0, -1.0),
        (x1, y1, -1.0, -1.0),
    ] {
        pb.move_to(cx, cy + sy * len);
        pb.line_to(cx, cy + sy * r);
        pb.cubic_to(cx, cy + sy * (r - k), cx + sx * (r - k), cy, cx + sx * r, cy);
        pb.line_to(cx + sx * len, cy);
    }
    if let Some(path) = pb.finish() {
        let stroke = Stroke {
            width: corner_w,
            line_cap: LineCap::Butt,
            line_join: LineJoin::Round,
            ..Stroke::default()
        };
        canvas.stroke_path(&path, &paint(color), &stroke, Transform::identity(), None);
    }
}

pub fn to_bgra(src: &[u8], dst: &mut [u8]) {
    for (s, d) in src.as_chunks::<4>().0.iter().zip(dst.as_chunks_mut::<4>().0) {
        *d = [s[2], s[1], s[0], s[3]];
    }
}

pub fn encode_png(pixmap: &Pixmap) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(pixmap.data().len() / 3);
    {
        let mut enc = png::Encoder::new(&mut out, pixmap.width(), pixmap.height());
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_compression(png::Compression::Fast);
        let mut writer = enc.write_header().map_err(|e| e.to_string())?;
        writer.write_image_data(pixmap.data()).map_err(|e| e.to_string())?;
    }
    Ok(out)
}
