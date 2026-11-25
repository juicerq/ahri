use std::path::PathBuf;

use smithay_client_toolkit::reexports::client::protocol::wl_shm;
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shell::wlr_layer::LayerSurface;
use smithay_client_toolkit::shm::slot::{Buffer, SlotPool};
use wayland_protocols::wp::viewporter::client::wp_viewport::WpViewport;

use smithay_client_toolkit::reexports::calloop::channel::Sender;
use smithay_client_toolkit::reexports::protocols::wp::cursor_shape::v1::client::wp_cursor_shape_device_v1::Shape;
use smithay_client_toolkit::seat::keyboard::Keysym;
use tiny_skia::{FillRule, PathBuilder, Pixmap, PixmapMut, Rect, Stroke, Transform};

use crate::media::{self, Event, FramePool, PlayRequest, Proc, Quality, Settings, Source, WAVE_BUCKETS};
use crate::text::{self, Canvas, Weight};
use crate::{Msg, draw, record};

pub const ZOOM: f32 = 1.3;
const MARGIN: f32 = 24.0;
const HEADER: f32 = 40.0;
const PAD: f32 = 12.0;
const GAP: f32 = 12.0;
const COL_W: f32 = 190.0;
const CARD_H: f32 = 56.0;
const CARD_GAP: f32 = 8.0;
const ROW_H: f32 = 28.0;
const ROW_GAP: f32 = 2.0;
const BTN_H: f32 = 36.0;
const SECTION: f32 = 22.0;
const BAR_GAP: f32 = 12.0;
const BAR_H: f32 = 16.0;
const LABELS_H: f32 = 18.0;
const TRIM_H: f32 = BAR_GAP + BAR_H + LABELS_H;
const MAX_VW: f32 = 520.0;
const MAX_VH: f32 = 340.0;
const MIN_VW: f32 = 320.0;
const MAX_UPSCALE: f32 = 2.0;
const MIN_CLIP: f32 = 0.5;
const GRAB: f32 = 8.0;
const VIDEO_RADIUS: f32 = 8.0;
const TRACK_NAMES: [&str; 2] = ["Sistema", "Microfone"];

const PANEL: [u8; 4] = [0x1b, 0x1d, 0x22, 255];
const BORDER: [u8; 4] = [0x2b, 0x2e, 0x35, 255];
const CARD: [u8; 4] = [0x25, 0x28, 0x2f, 255];
const SELECTED: [u8; 4] = [0x3a, 0x3f, 0x4c, 255];
const SCREEN: [u8; 4] = [0x0b, 0x0c, 0x0f, 255];
const TEXT: [u8; 4] = [0xe6, 0xe7, 0xea, 255];
const TEXT_DIM: [u8; 4] = [0x9a, 0x9e, 0xa8, 255];
const MUTED: [u8; 4] = [0x8a, 0x8e, 0x98, 255];
const SOFT: [u8; 4] = [0xc9, 0xcb, 0xd1, 255];
const YELLOW: [u8; 4] = [0xf5, 0xc5, 0x18, 255];
const INK: [u8; 4] = [0x14, 0x15, 0x18, 255];
const KNOB_OFF: [u8; 4] = [0x7d, 0x81, 0x8b, 255];
const SWITCH_OFF: [u8; 4] = [0x3a, 0x3d, 0x46, 255];
const WAVE_ON: [u8; 4] = [0xaa, 0xb0, 0xbd, 255];
const WAVE_OFF: [u8; 4] = [0x5c, 0x60, 0x6a, 255];
const RAIL: [u8; 4] = [0x2c, 0x2f, 0x37, 255];
const RANGE: [u8; 4] = [0x5a, 0x5f, 0x6b, 255];

#[derive(Debug, Clone, Copy, PartialEq)]
struct R {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

impl R {
    fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }
    fn right(&self) -> f32 {
        self.x + self.w
    }
    fn bottom(&self) -> f32 {
        self.y + self.h
    }
    fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x <= self.right() && y >= self.y && y <= self.bottom()
    }
    fn grow(&self, dx: f32, dy: f32) -> Self {
        Self::new(self.x - dx, self.y - dy, self.w + dx * 2.0, self.h + dy * 2.0)
    }
    fn scaled(&self, s: f32) -> Option<Rect> {
        Rect::from_xywh(self.x * s, self.y * s, self.w * s, self.h * s)
    }
}

#[derive(Debug, Clone, Copy)]
struct Layout {
    size: (u32, u32),
    panel: R,
    video_box: R,
    video: R,
    bar: R,
    col: R,
    cards: [R; 2],
    quality_label: f32,
    rows: [R; 3],
    button: R,
}

impl Layout {
    fn new(src: (u32, u32)) -> Self {
        let (sw, sh) = (src.0.max(1) as f32, src.1.max(1) as f32);
        let k = (MAX_VW / sw).min(MAX_VH / sh).min(MAX_UPSCALE);
        let (dw, dh) = ((sw * k).round().max(2.0), (sh * k).round().max(2.0));
        let col_h = SECTION + 2.0 * (CARD_H + CARD_GAP) + SECTION + 3.0 * (ROW_H + ROW_GAP) + GAP + BTN_H;
        let box_w = dw.max(MIN_VW);
        let box_h = dh.max(col_h - TRIM_H);
        let body_h = box_h + TRIM_H;
        let pw = PAD + box_w + GAP + COL_W + PAD;
        let ph = HEADER + PAD + body_h + PAD;
        let panel = R::new(MARGIN, MARGIN, pw, ph);
        let video_box = R::new(panel.x + PAD, panel.y + HEADER + PAD, box_w, box_h);
        let video = R::new(
            video_box.x + ((box_w - dw) / 2.0).floor(),
            video_box.y + ((box_h - dh) / 2.0).floor(),
            dw,
            dh,
        );
        let bar = R::new(video_box.x, video_box.bottom() + BAR_GAP, box_w, BAR_H);
        let col = R::new(video_box.right() + GAP, video_box.y, COL_W, body_h);
        let card = |i: f32| R::new(col.x, col.y + SECTION + i * (CARD_H + CARD_GAP), COL_W, CARD_H);
        let cards = [card(0.0), card(1.0)];
        let quality_label = cards[1].bottom() + CARD_GAP;
        let row = |i: f32| R::new(col.x, quality_label + SECTION + i * (ROW_H + ROW_GAP), COL_W, ROW_H);
        let rows = [row(0.0), row(1.0), row(2.0)];
        let button = R::new(col.x, col.bottom() - BTN_H, COL_W, BTN_H);
        Self {
            size: ((pw + MARGIN * 2.0) as u32, (ph + MARGIN * 2.0) as u32),
            panel,
            video_box,
            video,
            bar,
            col,
            cards,
            quality_label,
            rows,
            button,
        }
    }
}

struct Track {
    on: bool,
    peaks: Option<Vec<f32>>,
    missing: bool,
}

struct Player {
    id: u64,
    proc: Proc,
    still: bool,
}

struct Export {
    id: u64,
    key: Settings,
    proc: Option<Proc>,
    path: PathBuf,
    progress: f32,
    size: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Drag {
    From,
    To,
    Seek,
}

pub enum Outcome {
    Nothing,
    Redraw,
    Close,
    Copied(PathBuf),
    Failed(String),
}

pub struct Review {
    scale: f32,
    px: (u32, u32),
    lay: Layout,
    video_px: (usize, usize, usize, usize),
    base: Pixmap,
    chrome: Pixmap,
    base_dirty: bool,
    corners: Vec<u8>,
    source: Source,
    src_size: (u32, u32),
    tracks: Vec<Track>,
    quality: Quality,
    from: f32,
    to: f32,
    playing: bool,
    pos: f32,
    frames: u32,
    hold_end: bool,
    player: Option<Player>,
    still_wanted: Option<f32>,
    frame: Option<Vec<u8>>,
    pool: FramePool,
    export: Option<Export>,
    copying: bool,
    started: bool,
    drag: Option<(Drag, bool)>,
    tx: Sender<Msg>,
}

impl Review {
    pub fn new(scale: f32, raw: PathBuf, src_size: (u32, u32), duration: f32, tx: Sender<Msg>) -> Result<Self, String> {
        let scale = scale * ZOOM;
        let lay = Layout::new(src_size);
        let px = (
            (lay.size.0 as f32 * scale).round() as u32,
            (lay.size.1 as f32 * scale).round() as u32,
        );
        let base = Pixmap::new(px.0, px.1).ok_or_else(|| format!("panel {px:?}"))?;
        let chrome = chrome(&lay, scale, px)?;
        let bytes = raw.metadata().map(|m| m.len()).unwrap_or(0);
        let v = lay.video;
        let video_px = (
            (v.x * scale).round() as usize,
            (v.y * scale).round() as usize,
            ((v.w * scale).round() as usize).max(2),
            ((v.h * scale).round() as usize).max(2),
        );
        let corners = corner_coverage(VIDEO_RADIUS * scale);

        let mut review = Self {
            scale,
            px,
            lay,
            video_px,
            base,
            chrome,
            base_dirty: true,
            corners,
            source: Source {
                path: raw,
                duration: duration.max(MIN_CLIP),
                bytes,
            },
            src_size,
            tracks: (0..TRACK_NAMES.len())
                .map(|_| Track {
                    on: false,
                    peaks: None,
                    missing: false,
                })
                .collect(),
            quality: Quality::Light,
            from: 0.0,
            to: duration.max(MIN_CLIP),
            playing: false,
            pos: 0.0,
            frames: 0,
            hold_end: false,
            player: None,
            still_wanted: None,
            frame: None,
            pool: FramePool::default(),
            export: None,
            copying: false,
            started: false,
            drag: None,
            tx,
        };
        media::probe(&review.source.path, review.tx.clone());
        review.start_player(0.0);
        Ok(review)
    }

    pub fn input_rect(&self) -> (i32, i32, i32, i32) {
        let p = self.lay.panel;
        (
            (p.x * ZOOM).round() as i32,
            (p.y * ZOOM).round() as i32,
            (p.w * ZOOM).round() as i32,
            (p.h * ZOOM).round() as i32,
        )
    }

    fn settings(&self, quality: Quality) -> Settings {
        Settings {
            tracks: self.tracks.iter().map(|t| t.on && !t.missing).collect(),
            quality,
            from: self.from,
            to: self.to,
        }
    }

    fn export_key(&self) -> Settings {
        let mut s = self.settings(self.quality);
        if s.from <= 0.05 && s.to >= self.source.duration - 0.05 {
            s.from = 0.0;
            s.to = f32::MAX;
        }
        s
    }

    fn ensure_export(&mut self) {
        let key = self.export_key();
        if self.export.as_ref().is_some_and(|e| e.key == key) {
            return;
        }
        self.cancel_export();
        let id = media::next_id();
        let path = record::export_path(&self.source.path, id);
        let settings = self.settings(self.quality);
        let proc = match media::export(&self.source, &settings, &path, id, self.tx.clone()) {
            Ok(p) => Some(p),
            Err(e) => {
                eprintln!("ahri: export: {e}");
                None
            }
        };
        self.export = Some(Export {
            id,
            key,
            proc,
            path,
            progress: 0.0,
            size: None,
        });
        self.base_dirty = true;
    }

    fn cancel_export(&mut self) {
        if let Some(e) = self.export.take() {
            if let Some(p) = &e.proc {
                p.kill();
            }
            let _ = std::fs::remove_file(&e.path);
        }
    }

    fn start_player(&mut self, at: f32) {
        self.stop_player();
        let at = at.clamp(self.from, (self.to - 0.05).max(self.from));
        let id = media::next_id();
        let tracks = self.settings(self.quality).tracks;
        let req = PlayRequest {
            path: &self.source.path,
            from: at,
            to: self.to,
            size: (self.video_px.2 as u32, self.video_px.3 as u32),
            tracks: &tracks,
            still: false,
        };
        self.pos = at;
        self.frames = 0;
        self.hold_end = false;
        match media::play(req, id, self.pool.clone(), self.tx.clone()) {
            Ok(proc) => {
                self.player = Some(Player { id, proc, still: false });
                self.playing = true;
            }
            Err(e) => {
                eprintln!("ahri: preview: {e}");
                self.playing = false;
            }
        }
    }

    fn stop_player(&mut self) {
        if let Some(p) = self.player.take() {
            p.proc.kill();
        }
        self.frames = 0;
        self.hold_end = false;
        self.still_wanted = None;
    }

    fn pause(&mut self) {
        self.pos = self.current_pos();
        self.stop_player();
        self.playing = false;
    }

    fn current_pos(&self) -> f32 {
        if !self.playing {
            return self.pos;
        }
        if self.hold_end {
            return self.to;
        }
        (self.pos + self.frames as f32 / media::PREVIEW_FPS).min(self.to)
    }

    fn show_still(&mut self, at: f32) {
        if self.player.as_ref().is_some_and(|p| p.still) {
            self.still_wanted = Some(at);
            return;
        }
        let id = media::next_id();
        let req = PlayRequest {
            path: &self.source.path,
            from: at.min(self.source.duration - 0.02).max(0.0),
            to: self.source.duration,
            size: (self.video_px.2 as u32, self.video_px.3 as u32),
            tracks: &[],
            still: true,
        };
        match media::play(req, id, self.pool.clone(), self.tx.clone()) {
            Ok(proc) => self.player = Some(Player { id, proc, still: true }),
            Err(e) => eprintln!("ahri: still: {e}"),
        }
    }

    pub fn media(&mut self, ev: Event) -> Outcome {
        match ev {
            Event::Frame { id, data } => {
                let Some(player) = self.player.as_ref().filter(|p| p.id == id) else {
                    self.recycle(data);
                    return Outcome::Nothing;
                };
                if player.still {
                    self.player = None;
                    if let Some(at) = self.still_wanted.take() {
                        self.show_still(at);
                    }
                } else {
                    self.frames += 1;
                    self.hold_end = false;
                }
                if !self.started {
                    self.started = true;
                    media::load_waves(&self.source.path, self.tx.clone());
                    self.ensure_export();
                }
                if let Some(old) = self.frame.replace(data) {
                    self.recycle(old);
                }
                Outcome::Redraw
            }
            Event::Ended { id } => {
                if self.player.as_ref().is_some_and(|p| p.id == id && !p.still) {
                    self.player = None;
                    if self.drag.is_none() {
                        self.start_player(self.from);
                        self.hold_end = true;
                    }
                    return Outcome::Redraw;
                }
                Outcome::Nothing
            }
            Event::Duration { secs } => {
                if secs > MIN_CLIP && (secs - self.source.duration).abs() > 0.001 {
                    let full = self.to >= self.source.duration - 0.05;
                    self.source.duration = secs;
                    if full || self.to > secs {
                        self.to = secs;
                    }
                    self.from = self.from.min(self.to - MIN_CLIP).max(0.0);
                    self.pos = self.pos.min(self.to);
                    if self.started {
                        self.ensure_export();
                    }
                    self.base_dirty = true;
                    return Outcome::Redraw;
                }
                Outcome::Nothing
            }
            Event::Wave { track, peaks } => {
                let Some(t) = self.tracks.get_mut(track) else {
                    return Outcome::Nothing;
                };
                t.missing = peaks.is_none();
                t.peaks = peaks;
                if t.missing && t.on {
                    t.on = false;
                    if self.playing {
                        self.start_player(self.current_pos());
                    }
                }
                self.ensure_export();
                self.base_dirty = true;
                Outcome::Redraw
            }
            Event::Progress { id, done } => {
                let Some(e) = self.export.as_mut().filter(|e| e.id == id) else {
                    return Outcome::Nothing;
                };
                e.progress = done;
                if self.copying {
                    self.base_dirty = true;
                    return Outcome::Redraw;
                }
                Outcome::Nothing
            }
            Event::Exported { id, result } => {
                let Some(e) = self.export.as_mut().filter(|e| e.id == id) else {
                    return Outcome::Nothing;
                };
                e.proc = None;
                self.base_dirty = true;
                match result {
                    Ok(size) => {
                        e.size = Some(size);
                        if self.copying {
                            return self.finish();
                        }
                        Outcome::Redraw
                    }
                    Err(err) => {
                        eprintln!("ahri: export failed: {err}");
                        let _ = std::fs::remove_file(&e.path);
                        self.export = None;
                        if self.copying {
                            self.copying = false;
                            return Outcome::Failed(err);
                        }
                        Outcome::Redraw
                    }
                }
            }
        }
    }

    fn recycle(&self, buf: Vec<u8>) {
        if let Ok(mut p) = self.pool.lock()
            && p.len() < 4
        {
            p.push(buf);
        }
    }

    fn copy(&mut self) -> Outcome {
        if self.copying {
            return Outcome::Nothing;
        }
        if self.export.is_none() {
            self.ensure_export();
        }
        if self.export.as_ref().is_some_and(|e| e.size.is_some()) {
            return self.finish();
        }
        if self.export.as_ref().is_none_or(|e| e.proc.is_none()) {
            return Outcome::Failed("não foi possível preparar o vídeo".into());
        }
        self.copying = true;
        self.base_dirty = true;
        Outcome::Redraw
    }

    fn finish(&mut self) -> Outcome {
        let Some(export) = self.export.take() else {
            return Outcome::Nothing;
        };
        self.stop_player();
        let out = record::final_path(&self.source.path);
        if let Err(e) = std::fs::rename(&export.path, &out) {
            self.copying = false;
            return Outcome::Failed(format!("{}: {e}", out.display()));
        }
        let _ = std::fs::remove_file(&self.source.path);
        Outcome::Copied(out)
    }

    pub fn keep(&mut self) {
        self.stop_player();
        self.cancel_export();
        let to = record::final_path(&self.source.path);
        if let Err(e) = std::fs::rename(&self.source.path, &to) {
            eprintln!("ahri: keep {}: {e}", self.source.path.display());
        }
    }

    pub fn discard(&mut self) {
        self.stop_player();
        self.cancel_export();
        let _ = std::fs::remove_file(&self.source.path);
    }

    fn toggle_play(&mut self) -> Outcome {
        if self.playing {
            self.pause();
        } else {
            let at = if self.pos >= self.to - 0.05 {
                self.from
            } else {
                self.pos
            };
            self.start_player(at);
        }
        Outcome::Redraw
    }

    fn toggle_track(&mut self, i: usize) -> Outcome {
        let Some(t) = self.tracks.get_mut(i).filter(|t| !t.missing) else {
            return Outcome::Nothing;
        };
        t.on = !t.on;
        self.base_dirty = true;
        if self.playing {
            self.start_player(self.current_pos());
        }
        self.ensure_export();
        Outcome::Redraw
    }

    fn set_quality(&mut self, q: Quality) -> Outcome {
        if q == self.quality {
            return Outcome::Nothing;
        }
        self.quality = q;
        self.base_dirty = true;
        self.ensure_export();
        Outcome::Redraw
    }

    fn step_quality(&mut self, d: isize) -> Outcome {
        let i = Quality::ALL.iter().position(|&q| q == self.quality).unwrap_or(0) as isize;
        let n = (i + d).clamp(0, Quality::ALL.len() as isize - 1) as usize;
        self.set_quality(Quality::ALL[n])
    }

    fn set_trim(&mut self, which: Drag, t: f32) {
        match which {
            Drag::From => self.from = t.clamp(0.0, (self.to - MIN_CLIP).max(0.0)),
            Drag::To => self.to = t.clamp((self.from + MIN_CLIP).min(self.source.duration), self.source.duration),
            Drag::Seek => {}
        }
        self.base_dirty = true;
        self.ensure_export();
    }

    fn seek(&mut self, t: f32) -> Outcome {
        let t = t.clamp(self.from, self.to);
        if self.playing {
            self.start_player(t);
        } else {
            self.pos = t;
            self.show_still(t);
        }
        Outcome::Redraw
    }

    pub fn key(&mut self, key: Keysym, ctrl: bool) -> Outcome {
        if self.copying && key != Keysym::Escape {
            return Outcome::Nothing;
        }
        match key {
            Keysym::Escape => {
                self.discard();
                Outcome::Close
            }
            Keysym::c | Keysym::C if ctrl => self.copy(),
            Keysym::Return | Keysym::KP_Enter => self.copy(),
            Keysym::space => self.toggle_play(),
            Keysym::_1 | Keysym::KP_1 => self.toggle_track(0),
            Keysym::_2 | Keysym::KP_2 => self.toggle_track(1),
            Keysym::Up => self.step_quality(-1),
            Keysym::Down => self.step_quality(1),
            Keysym::Left => self.seek(self.current_pos() - 1.0),
            Keysym::Right => self.seek(self.current_pos() + 1.0),
            Keysym::i | Keysym::I if !ctrl => {
                let t = self.current_pos();
                self.set_trim(Drag::From, t);
                self.pos = self.pos.max(self.from);
                Outcome::Redraw
            }
            Keysym::o | Keysym::O if !ctrl => {
                let t = self.current_pos();
                self.set_trim(Drag::To, t);
                if self.playing {
                    self.start_player(self.current_pos().min(self.to));
                }
                Outcome::Redraw
            }
            _ => Outcome::Nothing,
        }
    }

    fn time_at(&self, x: f32) -> f32 {
        let b = self.lay.bar;
        ((x - b.x) / b.w).clamp(0.0, 1.0) * self.source.duration
    }

    fn x_at(&self, t: f32) -> f32 {
        let b = self.lay.bar;
        b.x + (t / self.source.duration).clamp(0.0, 1.0) * b.w
    }

    fn drag_at(&self, x: f32, y: f32) -> Option<Drag> {
        let zone = self.lay.bar.grow(GRAB, GRAB / 2.0);
        if !zone.contains(x, y) {
            return None;
        }
        let (df, dt) = ((x - self.x_at(self.from)).abs(), (x - self.x_at(self.to)).abs());
        if df <= GRAB && df <= dt {
            Some(Drag::From)
        } else if dt <= GRAB {
            Some(Drag::To)
        } else {
            Some(Drag::Seek)
        }
    }

    pub fn cursor_at(&self, x: f32, y: f32) -> Shape {
        if let Some((d, _)) = self.drag {
            return if d == Drag::Seek {
                Shape::Grabbing
            } else {
                Shape::EwResize
            };
        }
        match self.drag_at(x, y) {
            Some(Drag::Seek) => return Shape::Pointer,
            Some(_) => return Shape::EwResize,
            None => {}
        }
        let clickable = self.lay.video_box.contains(x, y)
            || self
                .lay
                .cards
                .iter()
                .zip(&self.tracks)
                .any(|(r, t)| !t.missing && r.contains(x, y))
            || self.lay.rows.iter().any(|r| r.contains(x, y))
            || self.lay.button.contains(x, y);
        if clickable { Shape::Pointer } else { Shape::Default }
    }

    pub fn press(&mut self, x: f32, y: f32) -> Outcome {
        if self.copying {
            return Outcome::Nothing;
        }
        if let Some(d) = self.drag_at(x, y) {
            let was_playing = self.playing;
            if was_playing {
                self.pause();
            }
            self.drag = Some((d, was_playing));
            return self.motion(x, y);
        }
        if self.lay.video_box.contains(x, y) {
            return self.toggle_play();
        }
        if let Some(i) = self.lay.cards.iter().position(|r| r.contains(x, y)) {
            return self.toggle_track(i);
        }
        if let Some(i) = self.lay.rows.iter().position(|r| r.contains(x, y)) {
            return self.set_quality(Quality::ALL[i]);
        }
        if self.lay.button.contains(x, y) {
            return self.copy();
        }
        Outcome::Nothing
    }

    pub fn motion(&mut self, x: f32, _y: f32) -> Outcome {
        let Some((d, _)) = self.drag else {
            return Outcome::Nothing;
        };
        let t = self.time_at(x);
        match d {
            Drag::From => {
                self.from = t.clamp(0.0, (self.to - MIN_CLIP).max(0.0));
                self.pos = self.from;
            }
            Drag::To => {
                self.to = t.clamp((self.from + MIN_CLIP).min(self.source.duration), self.source.duration);
                self.pos = self.to;
            }
            Drag::Seek => self.pos = t.clamp(self.from, self.to),
        }
        self.show_still(self.pos);
        Outcome::Redraw
    }

    pub fn release(&mut self) -> Outcome {
        let Some((d, was_playing)) = self.drag.take() else {
            return Outcome::Nothing;
        };
        if d != Drag::Seek {
            self.set_trim(d, if d == Drag::From { self.from } else { self.to });
            if d == Drag::To {
                self.pos = self.from;
            }
        }
        if was_playing {
            self.start_player(self.pos);
        }
        Outcome::Redraw
    }

    pub fn size(&self) -> (u32, u32) {
        (
            (self.lay.size.0 as f32 * ZOOM).round() as u32,
            (self.lay.size.1 as f32 * ZOOM).round() as u32,
        )
    }

    pub fn pixels(&self) -> (u32, u32) {
        self.px
    }

    pub fn render(&mut self, canvas: &mut [u8]) -> Option<(i32, i32, i32, i32)> {
        let full = self.base_dirty;
        if self.base_dirty {
            self.render_base();
            self.base_dirty = false;
        }
        canvas.copy_from_slice(self.base.data());
        self.render_dynamic(canvas);
        if full {
            return None;
        }
        let s = self.scale;
        let v = self.lay.video_box;
        let b = self.lay.bar.grow(GRAB, 0.0);
        let rect = R::new(v.x.min(b.x), v.y, v.w.max(b.w), b.bottom() + LABELS_H - v.y);
        Some((
            (rect.x * s) as i32 - 1,
            (rect.y * s) as i32 - 1,
            (rect.w * s) as i32 + 2,
            (rect.h * s) as i32 + 2,
        ))
    }

    fn render_dynamic(&self, data: &mut [u8]) {
        let s = self.scale;
        let (w, h) = self.px;
        let (vx, vy, vw, vh) = self.video_px;
        let stride = w as usize * 4;
        if let Some(frame) = self.frame.as_ref().filter(|f| f.len() == vw * vh * 4) {
            let row = vw * 4;
            for y in 0..vh {
                let dst = (vy + y) * stride + vx * 4;
                data[dst..dst + row].copy_from_slice(&frame[y * row..(y + 1) * row]);
            }
            self.round_video(data, stride);
        }
        let Some(mut pm) = PixmapMut::from_bytes(data, w, h) else {
            return;
        };
        let t = Transform::from_scale(s, s);
        if !self.playing && self.drag.is_none() {
            let v = self.lay.video;
            let (cx, cy) = (v.x + v.w / 2.0, v.y + v.h / 2.0);
            if let Some(c) = PathBuilder::from_circle(cx, cy, 20.0) {
                pm.fill_path(&c, &paint([0, 0, 0, 150]), FillRule::Winding, t, None);
            }
            let mut tri = PathBuilder::new();
            tri.move_to(cx - 5.0, cy - 8.0);
            tri.line_to(cx + 9.0, cy);
            tri.line_to(cx - 5.0, cy + 8.0);
            tri.close();
            if let Some(p) = tri.finish() {
                pm.fill_path(&p, &paint([255, 255, 255, 255]), FillRule::Winding, t, None);
            }
        }

        let b = self.lay.bar;
        let mid = b.y + b.h / 2.0;
        let (xf, xt) = (self.x_at(self.from), self.x_at(self.to));
        let pos = self.current_pos().clamp(self.from, self.to);
        let xp = self.x_at(pos);
        fill(&mut pm, R::new(b.x, mid - 2.0, b.w, 4.0), 2.0, RAIL, t);
        fill(&mut pm, R::new(xf, mid - 2.0, xt - xf, 4.0), 2.0, RANGE, t);
        if xp > xf {
            fill(&mut pm, R::new(xf, mid - 2.0, xp - xf, 4.0), 2.0, YELLOW, t);
        }
        fill(&mut pm, R::new(xf - 3.0, b.y, 6.0, b.h), 3.0, YELLOW, t);
        fill(&mut pm, R::new(xt - 3.0, b.y, 6.0, b.h), 3.0, YELLOW, t);
        fill(
            &mut pm,
            R::new(xp - 1.0, b.y + 2.0, 2.0, b.h - 4.0),
            1.0,
            [255, 255, 255, 255],
            t,
        );

        let mut c = Canvas {
            data,
            width: w,
            height: h,
        };
        let base = b.bottom() + 15.0;
        let clip = self.to - self.from;
        let center = if self.drag == Some((Drag::Seek, true)) || self.drag == Some((Drag::Seek, false)) {
            clock(pos)
        } else {
            seconds(clip)
        };
        label(
            &mut c,
            s,
            b.x,
            base,
            11.0,
            Weight::Regular,
            MUTED,
            &clock(self.from),
            Align::Left,
        );
        label(
            &mut c,
            s,
            b.x + b.w / 2.0,
            base,
            11.0,
            Weight::Regular,
            SOFT,
            &center,
            Align::Center,
        );
        label(
            &mut c,
            s,
            b.right(),
            base,
            11.0,
            Weight::Regular,
            MUTED,
            &clock(self.to),
            Align::Right,
        );
    }

    fn round_video(&self, data: &mut [u8], stride: usize) {
        let (vx, vy, vw, vh) = self.video_px;
        let s = self.scale;
        let bx = self.lay.video_box;
        let (bx0, by0) = ((bx.x * s).round() as isize, (bx.y * s).round() as isize);
        let (bx1, by1) = ((bx.right() * s).round() as isize, (bx.bottom() * s).round() as isize);
        let r = (self.corners.len() as f32).sqrt() as isize;
        let base = self.base.data();
        for (cx, cy, fx, fy) in [
            (bx0, by0, false, false),
            (bx1 - r, by0, true, false),
            (bx0, by1 - r, false, true),
            (bx1 - r, by1 - r, true, true),
        ] {
            for j in 0..r {
                for i in 0..r {
                    let (ci, cj) = (if fx { r - 1 - i } else { i }, if fy { r - 1 - j } else { j });
                    let a = self.corners[(cj * r + ci) as usize] as u32;
                    if a == 255 {
                        continue;
                    }
                    let (x, y) = (cx + i, cy + j);
                    if x < vx as isize || y < vy as isize || x >= (vx + vw) as isize || y >= (vy + vh) as isize {
                        continue;
                    }
                    let o = y as usize * stride + x as usize * 4;
                    for k in 0..4 {
                        data[o + k] = ((data[o + k] as u32 * a + base[o + k] as u32 * (255 - a)) / 255) as u8;
                    }
                }
            }
        }
    }

    fn render_base(&mut self) {
        let s = self.scale;
        let t = Transform::from_scale(s, s);
        let (w, h) = self.px;
        let lay = self.lay;
        let q_sizes: Vec<(Quality, String, bool)> = Quality::ALL
            .iter()
            .map(|&q| {
                let exact = self
                    .export
                    .as_ref()
                    .filter(|e| q == self.quality && e.key == self.export_key())
                    .and_then(|e| e.size);
                match exact {
                    Some(b) => (q, megabytes(b), true),
                    None => (
                        q,
                        format!("~{}", megabytes(self.source.estimate(&self.settings(q)))),
                        false,
                    ),
                }
            })
            .collect();
        self.base.data_mut().copy_from_slice(self.chrome.data());
        let mut pm = self.base.as_mut();

        for (i, (card, track)) in lay.cards.iter().zip(&self.tracks).enumerate() {
            fill(&mut pm, *card, 7.0, CARD, t);
            let on = track.on && !track.missing;
            let ink = if on { TEXT } else { MUTED };
            let (ix, iy) = (card.x + 10.0, card.y + 9.0);
            if i == 0 {
                speaker(&mut pm, ix, iy, ink, t);
            } else {
                mic(&mut pm, ix, iy, ink, t);
            }
            if !track.missing {
                let sw = R::new(card.right() - 10.0 - 24.0, card.y + 10.0, 24.0, 14.0);
                fill(&mut pm, sw, 7.0, if on { YELLOW } else { SWITCH_OFF }, t);
                let kx = if on { sw.right() - 7.0 } else { sw.x + 7.0 };
                if let Some(k) = PathBuilder::from_circle(kx, sw.y + 7.0, 5.0) {
                    pm.fill_path(
                        &k,
                        &paint(if on { PANEL } else { KNOB_OFF }),
                        FillRule::Winding,
                        t,
                        None,
                    );
                }
            }
            let area = R::new(card.x + 10.0, card.y + 31.0, card.w - 20.0, 18.0);
            let kept = (self.from / self.source.duration, self.to / self.source.duration);
            wave(&mut pm, area, track.peaks.as_deref(), on, kept, t);
        }

        for (row, &q) in lay.rows.iter().zip(&Quality::ALL) {
            if q == self.quality {
                fill(&mut pm, *row, 6.0, SELECTED, t);
            }
        }

        let btn = lay.button;
        fill(&mut pm, btn, 7.0, YELLOW, t);
        let progress = self.export.as_ref().map(|e| e.progress).unwrap_or(0.0);
        if self.copying {
            fill(
                &mut pm,
                R::new(btn.x + 6.0, btn.bottom() - 5.0, btn.w - 12.0, 2.0),
                1.0,
                [0, 0, 0, 50],
                t,
            );
            fill(
                &mut pm,
                R::new(btn.x + 6.0, btn.bottom() - 5.0, (btn.w - 12.0) * progress, 2.0),
                1.0,
                INK,
                t,
            );
        }
        let esc_w = keycap_width("Esc", s);
        let esc_x = lay.panel.right() - 14.0 - text::width("descartar", 12.0 * s, Weight::Regular) / s - 6.0 - esc_w;
        keycap_bg(&mut pm, esc_x, lay.panel.y + 12.0, esc_w, false, t);
        let ctrl_w = keycap_width("Ctrl C", s);
        let ctrl_x = btn.right() - 10.0 - ctrl_w;
        if !self.copying {
            keycap_bg(&mut pm, ctrl_x, btn.y + 10.0, ctrl_w, true, t);
        }

        let mut c = Canvas {
            data: self.base.data_mut(),
            width: w,
            height: h,
        };
        let hx = lay.panel.x + 14.0;
        let hb = lay.panel.y + 25.0;
        let after = label(&mut c, s, hx, hb, 13.0, Weight::Medium, TEXT, "Gravação", Align::Left);
        let meta = format!(
            "{} · {}×{} · 60 fps",
            clock(self.source.duration),
            self.src_size.0,
            self.src_size.1
        );
        label(
            &mut c,
            s,
            after + 8.0,
            hb,
            12.0,
            Weight::Regular,
            MUTED,
            &meta,
            Align::Left,
        );
        keycap_text(&mut c, s, esc_x, lay.panel.y + 12.0, "Esc", false);
        label(
            &mut c,
            s,
            lay.panel.right() - 14.0,
            hb,
            12.0,
            Weight::Regular,
            MUTED,
            "descartar",
            Align::Right,
        );

        let section = |c: &mut Canvas, y: f32, txt: &str| {
            label(c, s, lay.col.x, y + 10.0, 10.0, Weight::Medium, MUTED, txt, Align::Left);
        };
        section(&mut c, lay.col.y, "ÁUDIO");
        for (i, (card, track)) in lay.cards.iter().zip(&self.tracks).enumerate() {
            let on = track.on && !track.missing;
            label(
                &mut c,
                s,
                card.x + 30.0,
                card.y + 21.0,
                12.0,
                Weight::Regular,
                if on { TEXT } else { TEXT_DIM },
                TRACK_NAMES[i],
                Align::Left,
            );
            if track.missing {
                label(
                    &mut c,
                    s,
                    card.right() - 10.0,
                    card.y + 21.0,
                    11.0,
                    Weight::Regular,
                    MUTED,
                    "sem faixa",
                    Align::Right,
                );
            }
        }
        section(&mut c, lay.quality_label, "QUALIDADE");
        for (row, (q, size, exact)) in lay.rows.iter().zip(&q_sizes) {
            let sel = *q == self.quality;
            let by = row.y + 18.5;
            label(
                &mut c,
                s,
                row.x + 8.0,
                by,
                12.0,
                Weight::Regular,
                if sel { [255; 4] } else { TEXT_DIM },
                q.label(),
                Align::Left,
            );
            let color = if sel && *exact {
                TEXT
            } else if sel {
                SOFT
            } else {
                MUTED
            };
            label(
                &mut c,
                s,
                row.right() - 8.0,
                by,
                11.0,
                Weight::Regular,
                color,
                size,
                Align::Right,
            );
        }
        let bb = btn.y + 23.0;
        if self.copying {
            let txt = format!("Preparando {}%", (progress * 100.0).round() as u32);
            label(
                &mut c,
                s,
                btn.x + 12.0,
                bb,
                13.0,
                Weight::Medium,
                INK,
                &txt,
                Align::Left,
            );
        } else {
            label(
                &mut c,
                s,
                btn.x + 12.0,
                bb,
                13.0,
                Weight::Medium,
                INK,
                "Copiar",
                Align::Left,
            );
            keycap_text(&mut c, s, ctrl_x, btn.y + 10.0, "Ctrl C", true);
        }
    }
}

impl Drop for Review {
    fn drop(&mut self) {
        self.stop_player();
        if let Some(e) = &self.export
            && let Some(p) = &e.proc
        {
            p.kill();
        }
    }
}

fn paint(c: [u8; 4]) -> tiny_skia::Paint<'static> {
    draw::paint(draw::rgba(c[0], c[1], c[2], c[3]))
}

fn fill(pm: &mut PixmapMut, r: R, radius: f32, c: [u8; 4], t: Transform) {
    if r.w <= 0.0 || r.h <= 0.0 {
        return;
    }
    let path = if radius > 0.0 {
        draw::rounded(
            Rect::from_xywh(r.x, r.y, r.w, r.h).unwrap_or(Rect::from_xywh(0.0, 0.0, 1.0, 1.0).unwrap()),
            radius.min(r.w / 2.0).min(r.h / 2.0),
        )
    } else {
        Rect::from_xywh(r.x, r.y, r.w, r.h).map(PathBuilder::from_rect)
    };
    if let Some(p) = path {
        pm.fill_path(&p, &paint(c), FillRule::Winding, t, None);
    }
}

#[derive(Clone, Copy)]
enum Align {
    Left,
    Center,
    Right,
}

#[allow(clippy::too_many_arguments)]
fn label(
    c: &mut Canvas,
    s: f32,
    x: f32,
    baseline: f32,
    size: f32,
    weight: Weight,
    color: [u8; 4],
    txt: &str,
    align: Align,
) -> f32 {
    let px = size * s;
    let tw = text::width(txt, px, weight);
    let x = x * s;
    let start = match align {
        Align::Left => x,
        Align::Center => x - tw / 2.0,
        Align::Right => x - tw,
    };
    text::draw(c, start.round(), (baseline * s).round(), px, weight, color, txt) / s
}

const KEY_H: f32 = 16.0;
const KEY_PAD: f32 = 5.0;

fn keycap_width(txt: &str, s: f32) -> f32 {
    text::width(txt, 10.0 * s, Weight::Medium) / s + KEY_PAD * 2.0
}

fn keycap_bg(pm: &mut PixmapMut, x: f32, y: f32, w: f32, on_yellow: bool, t: Transform) {
    let r = R::new(x, y, w, KEY_H);
    if on_yellow {
        fill(pm, r, 4.0, [0, 0, 0, 40], t);
    } else {
        fill(pm, r, 4.0, [0x3a, 0x3d, 0x46, 255], t);
        fill(pm, r.grow(-1.0, -1.0), 3.0, [0x2c, 0x2f, 0x37, 255], t);
    }
}

fn keycap_text(c: &mut Canvas, s: f32, x: f32, y: f32, txt: &str, on_yellow: bool) {
    let color = if on_yellow { INK } else { [0xd7, 0xd9, 0xde, 255] };
    label(
        c,
        s,
        x + KEY_PAD,
        y + 11.5,
        10.0,
        Weight::Medium,
        color,
        txt,
        Align::Left,
    );
}

fn wave(pm: &mut PixmapMut, area: R, peaks: Option<&[f32]>, on: bool, kept: (f32, f32), t: Transform) {
    let step = 4.0;
    let n = (area.w / step).floor() as usize;
    let mid = area.y + area.h / 2.0;
    let color = match (peaks.is_some(), on) {
        (false, _) => SWITCH_OFF,
        (true, true) => WAVE_ON,
        (true, false) => WAVE_OFF,
    };
    let mut inside = PathBuilder::new();
    let mut outside = PathBuilder::new();
    for i in 0..n {
        let amp = peaks.map_or(0.0, |p| {
            let a = i * WAVE_BUCKETS / n;
            let b = ((i + 1) * WAVE_BUCKETS / n).max(a + 1).min(p.len());
            p[a.min(p.len())..b].iter().copied().fold(0.0, f32::max)
        });
        let bh = (loudness(amp) * area.h).clamp(1.5, area.h);
        let x = area.x + i as f32 * step + 0.8;
        let frac = (i as f32 + 0.5) / n as f32;
        let pb = if frac >= kept.0 && frac <= kept.1 {
            &mut inside
        } else {
            &mut outside
        };
        if let Some(r) = Rect::from_xywh(x, mid - bh / 2.0, 2.4, bh) {
            pb.push_rect(r);
        }
    }
    if let Some(p) = inside.finish() {
        pm.fill_path(&p, &paint(color), FillRule::Winding, t, None);
    }
    if let Some(p) = outside.finish() {
        pm.fill_path(&p, &paint(SWITCH_OFF), FillRule::Winding, t, None);
    }
}

fn speaker(pm: &mut PixmapMut, x: f32, y: f32, c: [u8; 4], t: Transform) {
    let mut body = PathBuilder::new();
    body.move_to(x + 1.0, y + 5.0);
    body.line_to(x + 4.0, y + 5.0);
    body.line_to(x + 7.5, y + 2.0);
    body.line_to(x + 7.5, y + 14.0);
    body.line_to(x + 4.0, y + 11.0);
    body.line_to(x + 1.0, y + 11.0);
    body.close();
    if let Some(p) = body.finish() {
        pm.fill_path(&p, &paint(c), FillRule::Winding, t, None);
    }
    let stroke = Stroke {
        width: 1.4,
        line_cap: tiny_skia::LineCap::Round,
        ..Stroke::default()
    };
    for (r, k) in [(3.0, 0.75), (6.0, 0.8)] {
        let mut arc = PathBuilder::new();
        let (cx, cy) = (x + 8.0, y + 8.0);
        let a = std::f32::consts::FRAC_PI_4 * k * 1.2;
        arc.move_to(cx + r * (-a).cos(), cy + r * (-a).sin());
        arc.quad_to(cx + r / a.cos(), cy, cx + r * a.cos(), cy + r * a.sin());
        if let Some(p) = arc.finish() {
            pm.stroke_path(&p, &paint(c), &stroke, t, None);
        }
    }
}

fn mic(pm: &mut PixmapMut, x: f32, y: f32, c: [u8; 4], t: Transform) {
    fill(pm, R::new(x + 5.0, y + 1.0, 6.0, 9.0), 3.0, c, t);
    let stroke = Stroke {
        width: 1.4,
        line_cap: tiny_skia::LineCap::Round,
        ..Stroke::default()
    };
    let mut pb = PathBuilder::new();
    pb.move_to(x + 2.5, y + 7.0);
    pb.cubic_to(x + 2.5, y + 13.0, x + 13.5, y + 13.0, x + 13.5, y + 7.0);
    pb.move_to(x + 8.0, y + 12.0);
    pb.line_to(x + 8.0, y + 15.0);
    if let Some(p) = pb.finish() {
        pm.stroke_path(&p, &paint(c), &stroke, t, None);
    }
}

fn chrome(lay: &Layout, s: f32, px: (u32, u32)) -> Result<Pixmap, String> {
    let mut chrome = Pixmap::new(px.0, px.1).ok_or_else(|| format!("panel {px:?}"))?;
    shadow(chrome.data_mut(), px, lay.panel, s);
    let mut pm = chrome.as_mut();
    let t = Transform::from_scale(s, s);
    fill(&mut pm, lay.panel, 12.0, PANEL, t);
    if let Some(p) = lay
        .panel
        .grow(-0.5, -0.5)
        .scaled(1.0)
        .and_then(|r| draw::rounded(r, 11.5))
    {
        let stroke = Stroke {
            width: 1.0,
            ..Stroke::default()
        };
        pm.stroke_path(&p, &paint(BORDER), &stroke, t, None);
    }
    fill(
        &mut pm,
        R::new(lay.panel.x + 1.0, lay.panel.y + HEADER, lay.panel.w - 2.0, 1.0),
        0.0,
        BORDER,
        t,
    );
    fill(&mut pm, lay.video_box, VIDEO_RADIUS, SCREEN, t);
    Ok(chrome)
}

fn shadow(data: &mut [u8], px: (u32, u32), panel: R, s: f32) {
    let (hw, hh) = (panel.w / 2.0, panel.h / 2.0);
    let (cx, cy) = (panel.x + hw, panel.y + hh + 6.0);
    let radius = 12.0;
    let spread = MARGIN - 4.0;
    let stride = px.0 as usize * 4;
    for (y, row) in data.chunks_exact_mut(stride).enumerate() {
        let qy = ((y as f32 + 0.5) / s - cy).abs() - hh + radius;
        for (x, d) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let qx = ((x as f32 + 0.5) / s - cx).abs() - hw + radius;
            let dist = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0) - radius;
            let k = 1.0 - (dist / spread).clamp(0.0, 1.0);
            d[3] = (k * k * 110.0) as u8;
        }
    }
}

pub fn loudness(peak: f32) -> f32 {
    if peak <= 0.0 {
        return 0.0;
    }
    ((20.0 * peak.log10() + 48.0) / 48.0).clamp(0.0, 1.0)
}

fn corner_coverage(radius: f32) -> Vec<u8> {
    let r = radius.ceil().max(1.0) as usize;
    let mut out = vec![255u8; r * r];
    for j in 0..r {
        for i in 0..r {
            let dx = radius - (i as f32 + 0.5);
            let dy = radius - (j as f32 + 0.5);
            if dx <= 0.0 || dy <= 0.0 {
                continue;
            }
            let d = (dx * dx + dy * dy).sqrt();
            out[j * r + i] = ((radius - d + 0.5).clamp(0.0, 1.0) * 255.0) as u8;
        }
    }
    out
}

pub fn clock(t: f32) -> String {
    let t = t.max(0.0).floor() as u32;
    format!("{}:{:02}", t / 60, t % 60)
}

pub fn seconds(t: f32) -> String {
    let t = t.max(0.0);
    if t < 10.0 {
        format!("{:.1} s", t).replace('.', ",")
    } else {
        format!("{} s", t.round() as u32)
    }
}

pub fn megabytes(bytes: u64) -> String {
    let mb = bytes as f64 / 1_000_000.0;
    if mb < 0.1 {
        format!("{} KB", (bytes as f64 / 1000.0).round().max(1.0) as u64)
    } else if mb < 10.0 {
        format!("{mb:.1} MB").replace('.', ",")
    } else {
        format!("{} MB", mb.round() as u64)
    }
}

pub struct Panel {
    pub layer: LayerSurface,
    pub viewport: WpViewport,
    pub review: Review,
    pub frame_pending: bool,
    pub dirty: bool,
    pub configured: bool,
    buffers: Vec<Buffer>,
}

impl Panel {
    pub fn new(layer: LayerSurface, viewport: WpViewport, review: Review) -> Self {
        let (w, h) = review.size();
        layer.set_size(w, h);
        viewport.set_destination(w as i32, h as i32);
        Self {
            layer,
            viewport,
            review,
            frame_pending: false,
            dirty: true,
            configured: false,
            buffers: Vec::new(),
        }
    }

    pub fn present(&mut self, pool: &mut SlotPool) -> Result<(), String> {
        let (w, h) = self.review.pixels();
        let idx = match (0..self.buffers.len()).find(|&i| self.buffers[i].canvas(pool).is_some()) {
            Some(i) => i,
            None => {
                let (buffer, _) = pool
                    .create_buffer(w as i32, h as i32, w as i32 * 4, wl_shm::Format::Argb8888)
                    .map_err(|e| format!("create buffer: {e}"))?;
                self.buffers.push(buffer);
                self.buffers.len() - 1
            }
        };
        let canvas = self.buffers[idx].canvas(pool).ok_or("buffer busy")?;
        let damage = self.review.render(canvas);
        let surface = self.layer.wl_surface();
        self.buffers[idx]
            .attach_to(surface)
            .map_err(|e| format!("attach: {e:?}"))?;
        let (x, y, dw, dh) = damage.unwrap_or((0, 0, w as i32, h as i32));
        surface.damage_buffer(x, y, dw, dh);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_fits_video_and_column() {
        for src in [(1920, 1080), (400, 1200), (120, 80)] {
            let lay = Layout::new(src);
            assert!(lay.video.w <= lay.video_box.w && lay.video.h <= lay.video_box.h);
            assert!(lay.video.w <= src.0 as f32 * MAX_UPSCALE + 1.0);
            assert!(lay.button.bottom() <= lay.panel.bottom() - PAD + 0.01);
            assert!(lay.rows[2].bottom() < lay.button.y);
            assert!(lay.bar.bottom() + LABELS_H <= lay.panel.bottom() - PAD + 0.01);
            assert_eq!(lay.size.0 as f32, lay.panel.w + MARGIN * 2.0);
        }
    }

    #[test]
    fn labels_read_in_portuguese() {
        assert_eq!(clock(74.9), "1:14");
        assert_eq!(seconds(4.26), "4,3 s");
        assert_eq!(seconds(12.6), "13 s");
        assert_eq!(megabytes(6_100_000), "6,1 MB");
        assert_eq!(megabytes(18_400_000), "18 MB");
        assert_eq!(megabytes(42_000), "42 KB");
    }

    #[test]
    fn loudness_maps_decibels() {
        assert_eq!(loudness(0.0), 0.0);
        assert_eq!(loudness(1.0), 1.0);
        assert!((loudness(10f32.powf(-24.0 / 20.0)) - 0.5).abs() < 0.001);
    }
}
