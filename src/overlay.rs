use smithay_client_toolkit::reexports::client::protocol::{wl_output::WlOutput, wl_shm};
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shell::wlr_layer::LayerSurface;
use smithay_client_toolkit::shm::slot::{Buffer, SlotPool};
use tiny_skia::{BlendMode, Color, FillRule, Mask, Paint, PathBuilder, Pixmap, Point, Rect, Stroke, Transform};
use wayland_protocols::wp::viewporter::client::wp_viewport::WpViewport;

use smithay_client_toolkit::reexports::protocols::wp::cursor_shape::v1::client::wp_cursor_shape_device_v1::Shape;

use crate::draw::{self, Mark, Sel, Tool};

pub enum Mode {
    Shot { shot: Pixmap, dim: Pixmap },
    Record,
}

pub enum Drag {
    None,
    Selecting(Point),
    Marking,
    Resizing(Sel, Edges),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Edges {
    pub left: bool,
    pub right: bool,
    pub top: bool,
    pub bottom: bool,
}

impl Edges {
    fn any(&self) -> bool {
        self.left || self.right || self.top || self.bottom
    }

    pub fn cursor(&self) -> Shape {
        match (self.left, self.right, self.top, self.bottom) {
            (true, _, true, _) | (_, true, _, true) => Shape::NwseResize,
            (true, _, _, true) | (_, true, true, _) => Shape::NeswResize,
            (true, _, _, _) | (_, true, _, _) => Shape::EwResize,
            _ => Shape::NsResize,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hit {
    Tool(Tool),
    ColorButton,
    Swatch(usize),
    Panel,
}

pub enum Outcome {
    Nothing,
    Redraw,
    Close,
    Copy,
    Record(Sel),
}

const BTN: f32 = 36.0;
const PAD: f32 = 6.0;
const MARK_WIDTH: f32 = 4.0;
const MIN_SEL: f32 = 4.0;
const RING_GAP: f32 = 4.0;
const GRAB: f32 = 8.0;

pub struct Overlay {
    pub layer: LayerSurface,
    pub viewport: WpViewport,
    pub mode: Mode,
    pub output: Option<WlOutput>,
    pub logical: (u32, u32),
    pub scale: f32,
    pub frame: Option<Pixmap>,
    buffers: Vec<Buffer>,
    pub sel: Option<Sel>,
    pub drag: Drag,
    pub marks: Vec<Mark>,
    pub current: Option<Mark>,
    pub tool: Tool,
    color: usize,
    palette_open: bool,
    pub frame_pending: bool,
    pub dirty: bool,
    mask: Option<(Sel, Mask)>,
    ring: Option<Sel>,
}

impl Overlay {
    pub fn new(layer: LayerSurface, viewport: WpViewport, mode: Mode) -> Self {
        Self {
            layer,
            viewport,
            mode,
            output: None,
            logical: (0, 0),
            scale: 1.0,
            frame: None,
            buffers: Vec::new(),
            sel: None,
            drag: Drag::None,
            marks: Vec::new(),
            current: None,
            tool: Tool::Draw,
            color: 0,
            palette_open: false,
            frame_pending: false,
            dirty: true,
            mask: None,
            ring: None,
        }
    }

    pub fn is_shot(&self) -> bool {
        matches!(self.mode, Mode::Shot { .. })
    }

    pub fn configure(&mut self, w: u32, h: u32) {
        self.logical = (w, h);
        let (bw, bh) = match &self.mode {
            Mode::Shot { shot, .. } => (shot.width(), shot.height()),
            Mode::Record => (w, h),
        };
        self.scale = bw as f32 / w.max(1) as f32;
        if self.frame.as_ref().is_none_or(|f| f.width() != bw || f.height() != bh) {
            self.frame = Pixmap::new(bw, bh);
            self.buffers.clear();
        }
        self.viewport.set_destination(w as i32, h as i32);
        self.dirty = true;
    }

    pub fn to_buffer(&self, pos: (f64, f64)) -> Point {
        Point::from_xy(pos.0 as f32 * self.scale, pos.1 as f32 * self.scale)
    }

    pub fn press(&mut self, p: Point) -> Outcome {
        if let Some(hit) = self.hit(p) {
            match hit {
                Hit::Tool(tool) => {
                    self.tool = tool;
                    self.palette_open = false;
                }
                Hit::ColorButton => self.palette_open = !self.palette_open,
                Hit::Swatch(i) => {
                    self.color = i;
                    self.palette_open = false;
                }
                Hit::Panel => {}
            }
            return Outcome::Redraw;
        }
        self.palette_open = false;
        if let Some((sel, edges)) = self.edges_at(p) {
            self.drag = Drag::Resizing(sel, edges);
            return Outcome::Nothing;
        }
        if self.is_shot() && self.sel.is_some_and(|s| s.contains(p)) {
            self.current = Some(match self.tool {
                Tool::Draw => Mark::Draw(vec![p], draw::palette(self.color)),
                Tool::Arrow => Mark::Arrow(p, p, draw::palette(self.color)),
            });
            self.drag = Drag::Marking;
        } else {
            self.drag = Drag::Selecting(p);
            self.sel = Some(Sel::from_points(p, p));
        }
        Outcome::Redraw
    }

    pub fn motion(&mut self, p: Point) -> Outcome {
        match &self.drag {
            Drag::None => return Outcome::Nothing,
            Drag::Selecting(start) => self.sel = Some(Sel::from_points(*start, p)),
            Drag::Resizing(orig, edges) => {
                let (mut x0, mut y0, mut x1, mut y1) = (orig.x, orig.y, orig.x + orig.w, orig.y + orig.h);
                if edges.left {
                    x0 = p.x;
                }
                if edges.right {
                    x1 = p.x;
                }
                if edges.top {
                    y0 = p.y;
                }
                if edges.bottom {
                    y1 = p.y;
                }
                self.sel = Some(Sel::from_points(Point::from_xy(x0, y0), Point::from_xy(x1, y1)));
            }
            Drag::Marking => match self.current.as_mut() {
                Some(Mark::Draw(points, _)) => points.push(p),
                Some(Mark::Arrow(_, end, _)) => *end = p,
                None => {}
            },
        }
        Outcome::Redraw
    }

    pub fn release(&mut self) -> Outcome {
        let drag = std::mem::replace(&mut self.drag, Drag::None);
        match drag {
            Drag::None => Outcome::Nothing,
            Drag::Marking => {
                if let Some(mark) = self.current.take() {
                    self.marks.push(mark);
                }
                Outcome::Redraw
            }
            Drag::Resizing(orig, _) => {
                let min = MIN_SEL * self.scale;
                if self.sel.is_none_or(|s| s.w < min || s.h < min) {
                    self.sel = Some(orig);
                }
                Outcome::Redraw
            }
            Drag::Selecting(_) => {
                let min = MIN_SEL * self.scale;
                match self.sel {
                    Some(sel) if sel.w >= min && sel.h >= min => {
                        if self.is_shot() {
                            Outcome::Redraw
                        } else {
                            Outcome::Record(sel)
                        }
                    }
                    _ => {
                        self.sel = None;
                        Outcome::Redraw
                    }
                }
            }
        }
    }

    fn edges_at(&self, p: Point) -> Option<(Sel, Edges)> {
        if !self.is_shot() {
            return None;
        }
        let sel = self.sel?;
        let g = GRAB * self.scale;
        let (x0, y0, x1, y1) = (sel.x, sel.y, sel.x + sel.w, sel.y + sel.h);
        if p.x < x0 - g || p.x > x1 + g || p.y < y0 - g || p.y > y1 + g {
            return None;
        }
        let edges = Edges {
            left: (p.x - x0).abs() <= g,
            right: (p.x - x1).abs() <= g,
            top: (p.y - y0).abs() <= g,
            bottom: (p.y - y1).abs() <= g,
        };
        let edges = Edges {
            left: edges.left && !(edges.right && (p.x - x1).abs() < (p.x - x0).abs()),
            right: edges.right && !(edges.left && (p.x - x0).abs() <= (p.x - x1).abs()),
            top: edges.top && !(edges.bottom && (p.y - y1).abs() < (p.y - y0).abs()),
            bottom: edges.bottom && !(edges.top && (p.y - y0).abs() <= (p.y - y1).abs()),
        };
        edges.any().then_some((sel, edges))
    }

    pub fn cursor_at(&self, p: Point) -> Shape {
        if let Drag::Resizing(_, edges) = &self.drag {
            return edges.cursor();
        }
        if !matches!(self.drag, Drag::None) {
            return Shape::Crosshair;
        }
        if self.hit(p).is_some() {
            return Shape::Default;
        }
        match self.edges_at(p) {
            Some((_, edges)) => edges.cursor(),
            None => Shape::Crosshair,
        }
    }

    pub fn arm(&mut self, capture: Sel) {
        self.ring = Some(capture);
        self.sel = None;
        self.drag = Drag::None;
        self.dirty = true;
    }

    pub fn set_tool(&mut self, tool: Tool) -> Outcome {
        self.tool = tool;
        self.palette_open = false;
        Outcome::Redraw
    }

    pub fn toggle_palette(&mut self) -> Outcome {
        self.palette_open = !self.palette_open;
        Outcome::Redraw
    }

    pub fn number(&mut self, n: usize) -> Outcome {
        if self.palette_open {
            if n >= 1 && n <= draw::PALETTE.len() {
                self.color = n - 1;
                self.palette_open = false;
                return Outcome::Redraw;
            }
            return Outcome::Nothing;
        }
        match n {
            1 => self.set_tool(Tool::Draw),
            2 => self.set_tool(Tool::Arrow),
            3 => self.toggle_palette(),
            _ => Outcome::Nothing,
        }
    }

    pub fn next_color(&mut self) -> Outcome {
        self.color = (self.color + 1) % draw::PALETTE.len();
        Outcome::Redraw
    }

    pub fn undo(&mut self) -> Outcome {
        self.marks.pop();
        Outcome::Redraw
    }

    pub fn export(&self) -> Option<Pixmap> {
        let Mode::Shot { shot, .. } = &self.mode else {
            return None;
        };
        let (x, y, w, h) = match self.sel {
            Some(sel) => sel.snapped(shot.width(), shot.height()),
            None => (0, 0, shot.width(), shot.height()),
        };
        let mut out = draw::crop(shot, x, y, w, h)?;
        let t = Transform::from_translate(-(x as f32), -(y as f32));
        for mark in &self.marks {
            draw::draw_mark(&mut out, mark, MARK_WIDTH * self.scale, t, None);
        }
        Some(out)
    }

    fn toolbar_rect(&self) -> Option<Rect> {
        if !self.is_shot() || matches!(self.drag, Drag::Selecting(_)) {
            return None;
        }
        let sel = self.sel?;
        let Mode::Shot { shot, .. } = &self.mode else {
            return None;
        };
        let s = self.scale;
        let (w, h) = ((BTN * 3.0 + PAD * 4.0) * s, (BTN + PAD * 2.0) * s);
        let gap = 8.0 * s;
        let (fw, fh) = (shot.width() as f32, shot.height() as f32);
        let x = (sel.x + sel.w - w).clamp(0.0, fw - w);
        let y = if sel.y + sel.h + gap + h <= fh {
            sel.y + sel.h + gap
        } else if sel.y - gap - h >= 0.0 {
            sel.y - gap - h
        } else {
            sel.y + sel.h - h - gap
        };
        Rect::from_xywh(x, y, w, h)
    }

    fn button_rects(&self) -> Option<[(Hit, Rect); 3]> {
        let bar = self.toolbar_rect()?;
        let s = self.scale;
        let b = |i: f32| {
            Rect::from_xywh(
                bar.x() + (PAD + i * (BTN + PAD)) * s,
                bar.y() + PAD * s,
                BTN * s,
                BTN * s,
            )
        };
        Some([
            (Hit::Tool(Tool::Draw), b(0.0)?),
            (Hit::Tool(Tool::Arrow), b(1.0)?),
            (Hit::ColorButton, b(2.0)?),
        ])
    }

    fn swatch_rects(&self) -> Option<Vec<(Hit, Rect)>> {
        if !self.palette_open {
            return None;
        }
        let bar = self.toolbar_rect()?;
        let Mode::Shot { shot, .. } = &self.mode else {
            return None;
        };
        let s = self.scale;
        let n = draw::PALETTE.len() as f32;
        let (w, h) = ((BTN * n + PAD * (n + 1.0)) * s, bar.height());
        let gap = 6.0 * s;
        let x = (bar.right() - w).clamp(0.0, shot.width() as f32 - w);
        let y = if bar.bottom() + gap + h <= shot.height() as f32 {
            bar.bottom() + gap
        } else {
            bar.top() - gap - h
        };
        (0..draw::PALETTE.len())
            .map(|i| {
                let r = Rect::from_xywh(x + (PAD + i as f32 * (BTN + PAD)) * s, y + PAD * s, BTN * s, BTN * s)?;
                Some((Hit::Swatch(i), r))
            })
            .collect()
    }

    fn palette_rect(&self) -> Option<Rect> {
        let swatches = self.swatch_rects()?;
        let (first, last) = (swatches.first()?.1, swatches.last()?.1);
        let pad = PAD * self.scale;
        Rect::from_ltrb(
            first.left() - pad,
            first.top() - pad,
            last.right() + pad,
            last.bottom() + pad,
        )
    }

    fn hit(&self, p: Point) -> Option<Hit> {
        let inside = |r: &Rect| p.x >= r.left() && p.x <= r.right() && p.y >= r.top() && p.y <= r.bottom();
        self.swatch_rects()
            .into_iter()
            .flatten()
            .chain(self.button_rects().into_iter().flatten())
            .find(|(_, r)| inside(r))
            .map(|(h, _)| h)
            .or_else(|| self.palette_rect().filter(inside).map(|_| Hit::Panel))
    }

    pub fn render(&mut self) {
        let Some(mut frame) = self.frame.take() else { return };
        match &self.mode {
            Mode::Shot { shot, dim } => {
                frame.data_mut().copy_from_slice(dim.data());
                if let Some(sel) = self.sel {
                    let (x, y, w, h) = sel.snapped(shot.width(), shot.height());
                    draw::copy_rect(&mut frame, shot, x, y, w, h);
                    self.ensure_mask(sel, frame.width(), frame.height());
                    let mask = self.mask.as_ref().map(|(_, m)| m);
                    let width = MARK_WIDTH * self.scale;
                    for mark in self.marks.iter().chain(self.current.iter()) {
                        draw::draw_mark(&mut frame, mark, width, Transform::identity(), mask);
                    }
                    draw::selection_frame(&mut frame, &sel, 0.0, draw::frame_white(), self.scale);
                }
                self.render_toolbar(&mut frame);
            }
            Mode::Record if self.ring.is_some() => {
                frame.fill(Color::TRANSPARENT);
                let ring = self.ring.unwrap();
                draw::selection_frame(&mut frame, &ring, RING_GAP, draw::frame_red(), self.scale);
            }
            Mode::Record => {
                frame.fill(Color::from_rgba8(0, 0, 0, 90));
                if let Some(rect) = self.sel.and_then(|s| s.rect()) {
                    let clear = Paint {
                        blend_mode: BlendMode::Clear,
                        ..Paint::default()
                    };
                    frame.fill_rect(rect, &clear, Transform::identity(), None);
                    draw::selection_frame(&mut frame, &self.sel.unwrap(), 0.0, draw::frame_white(), self.scale);
                }
            }
        }
        self.frame = Some(frame);
    }

    fn ensure_mask(&mut self, sel: Sel, w: u32, h: u32) {
        if self.mask.as_ref().is_some_and(|(s, _)| *s == sel) {
            return;
        }
        self.mask = Mask::new(w, h).zip(sel.rect()).map(|(mut mask, rect)| {
            mask.fill_path(
                &PathBuilder::from_rect(rect),
                FillRule::Winding,
                false,
                Transform::identity(),
            );
            (sel, mask)
        });
    }

    fn render_toolbar(&self, frame: &mut Pixmap) {
        let (Some(bar), Some(buttons)) = (self.toolbar_rect(), self.button_rects()) else {
            return;
        };
        let s = self.scale;
        let id = Transform::identity();
        let bg = draw::paint(Color::from_rgba8(32, 34, 37, 240));
        let active = draw::paint(Color::from_rgba8(255, 255, 255, 40));
        let current = draw::palette(self.color);
        if let Some(path) = rounded(bar, 8.0 * s) {
            frame.fill_path(&path, &bg, FillRule::Winding, id, None);
        }
        for (hit, r) in buttons {
            let on = match hit {
                Hit::Tool(tool) => tool == self.tool,
                _ => self.palette_open,
            };
            if on && let Some(path) = rounded(r, 6.0 * s) {
                frame.fill_path(&path, &active, FillRule::Winding, id, None);
            }
            let inset = 9.0 * s;
            let (x0, y0, x1, y1) = (r.left() + inset, r.top() + inset, r.right() - inset, r.bottom() - inset);
            let icon = match hit {
                Hit::Tool(Tool::Draw) => {
                    let n = 12;
                    Mark::Draw(
                        (0..=n)
                            .map(|i| {
                                let t = i as f32 / n as f32;
                                let y = (y0 + y1) / 2.0 + (t * std::f32::consts::TAU).sin() * (y1 - y0) * 0.3;
                                Point::from_xy(x0 + t * (x1 - x0), y)
                            })
                            .collect(),
                        current,
                    )
                }
                Hit::Tool(Tool::Arrow) => Mark::Arrow(Point::from_xy(x0, y1), Point::from_xy(x1, y0), current),
                _ => {
                    swatch(frame, r, current, false, s);
                    continue;
                }
            };
            draw::draw_mark(frame, &icon, 2.5 * s, id, None);
        }

        let (Some(panel), Some(swatches)) = (self.palette_rect(), self.swatch_rects()) else {
            return;
        };
        if let Some(path) = rounded(panel, 8.0 * s) {
            frame.fill_path(&path, &bg, FillRule::Winding, id, None);
        }
        for (hit, r) in swatches {
            if let Hit::Swatch(i) = hit {
                swatch(frame, r, draw::palette(i), i == self.color, s);
            }
        }
    }

    pub fn present(&mut self, pool: &mut SlotPool) -> Result<(), String> {
        let Some(frame) = self.frame.as_ref() else {
            return Ok(());
        };
        let (w, h) = (frame.width() as i32, frame.height() as i32);
        let idx = match (0..self.buffers.len()).find(|&i| self.buffers[i].canvas(pool).is_some()) {
            Some(i) => i,
            None => {
                let (buffer, _) = pool
                    .create_buffer(w, h, w * 4, wl_shm::Format::Argb8888)
                    .map_err(|e| format!("create buffer: {e}"))?;
                self.buffers.push(buffer);
                self.buffers.len() - 1
            }
        };
        let buffer = &self.buffers[idx];
        let canvas = buffer.canvas(pool).ok_or("buffer busy")?;
        draw::to_bgra(frame.data(), canvas);
        let surface = self.layer.wl_surface();
        buffer.attach_to(surface).map_err(|e| format!("attach: {e:?}"))?;
        surface.damage_buffer(0, 0, w, h);
        Ok(())
    }
}

fn swatch(frame: &mut Pixmap, r: Rect, color: Color, selected: bool, s: f32) {
    let (cx, cy) = ((r.left() + r.right()) / 2.0, (r.top() + r.bottom()) / 2.0);
    let id = Transform::identity();
    if let Some(dot) = PathBuilder::from_circle(cx, cy, 8.0 * s) {
        frame.fill_path(&dot, &draw::paint(color), FillRule::Winding, id, None);
        let edge = Stroke {
            width: 1.0 * s,
            ..Stroke::default()
        };
        frame.stroke_path(
            &dot,
            &draw::paint(Color::from_rgba8(255, 255, 255, 70)),
            &edge,
            id,
            None,
        );
    }
    if selected && let Some(ring) = PathBuilder::from_circle(cx, cy, 12.0 * s) {
        let stroke = Stroke {
            width: 2.0 * s,
            ..Stroke::default()
        };
        frame.stroke_path(&ring, &draw::paint(Color::WHITE), &stroke, id, None);
    }
}

fn rounded(r: Rect, radius: f32) -> Option<tiny_skia::Path> {
    let (x, y, w, h) = (r.x(), r.y(), r.width(), r.height());
    let k = radius * 0.5523;
    let mut pb = PathBuilder::new();
    pb.move_to(x + radius, y);
    pb.line_to(x + w - radius, y);
    pb.cubic_to(x + w - radius + k, y, x + w, y + radius - k, x + w, y + radius);
    pb.line_to(x + w, y + h - radius);
    pb.cubic_to(
        x + w,
        y + h - radius + k,
        x + w - radius + k,
        y + h,
        x + w - radius,
        y + h,
    );
    pb.line_to(x + radius, y + h);
    pb.cubic_to(x + radius - k, y + h, x, y + h - radius + k, x, y + h - radius);
    pb.line_to(x, y + radius);
    pb.cubic_to(x, y + radius - k, x + radius - k, y, x + radius, y);
    pb.close();
    pb.finish()
}
