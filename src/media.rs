use std::io::{BufRead, BufReader, Read};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use smithay_client_toolkit::reexports::calloop::channel::Sender;

use crate::Msg;

pub const WAVE_BUCKETS: usize = 240;
pub const PREVIEW_FPS: f32 = 60.0;
const WAVE_RATE: usize = 8000;
const TRACKS: usize = 2;
const MIC: usize = 1;
const DENOISE: &str = "highpass=f=80,afftdn=nr=12:nf=-35";

pub enum Event {
    Frame { id: u64, data: Vec<u8> },
    Ended { id: u64 },
    Wave { track: usize, peaks: Option<Vec<f32>> },
    Duration { secs: f32 },
    Progress { id: u64, done: f32 },
    Exported { id: u64, result: Result<u64, String> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quality {
    Original,
    Light,
    Minimal,
}

impl Quality {
    pub const ALL: [Quality; 3] = [Quality::Original, Quality::Light, Quality::Minimal];

    pub fn label(self) -> &'static str {
        match self {
            Quality::Original => "Original",
            Quality::Light => "Leve",
            Quality::Minimal => "Mínima",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub tracks: Vec<bool>,
    pub quality: Quality,
    pub from: f32,
    pub to: f32,
}

pub struct Source {
    pub path: PathBuf,
    pub duration: f32,
    pub bytes: u64,
}

impl Source {
    fn video_bps(&self) -> f32 {
        let total = self.bytes as f32 * 8.0 / self.duration.max(0.1);
        (total - 220_000.0).max(60_000.0)
    }

    fn trimmed(&self, s: &Settings) -> bool {
        s.from > 0.05 || s.to < self.duration - 0.05
    }

    fn target_bps(&self, q: Quality) -> f32 {
        match q {
            Quality::Original => self.video_bps(),
            Quality::Light => (self.video_bps() * 0.6).clamp(150_000.0, 2_500_000.0),
            Quality::Minimal => (self.video_bps() * 0.3).clamp(80_000.0, 900_000.0),
        }
    }

    fn audio_bps(q: Quality) -> u32 {
        if q == Quality::Minimal { 96_000 } else { 128_000 }
    }

    pub fn estimate(&self, s: &Settings) -> u64 {
        let secs = (s.to - s.from).max(0.0);
        let audio = if s.tracks.iter().any(|&t| t) {
            Self::audio_bps(s.quality) as f32
        } else {
            0.0
        };
        ((self.target_bps(s.quality) + audio) * secs / 8.0) as u64
    }

    pub fn export_args(&self, s: &Settings, out: &Path) -> Vec<String> {
        let mut a: Vec<String> = ["-nostdin", "-v", "error", "-y"].map(String::from).to_vec();
        let trimmed = self.trimmed(s);
        if trimmed {
            a.extend(["-ss".into(), secs(s.from), "-t".into(), secs(s.to - s.from)]);
        }
        a.extend(["-i".into(), self.path.to_string_lossy().into_owned()]);
        a.extend(["-map", "0:v:0"].map(String::from));
        let rate = |bps: f32| {
            [
                "-b:v".to_string(),
                format!("{}", bps as u32),
                "-maxrate".into(),
                format!("{}", (bps * 1.5) as u32),
                "-bufsize".into(),
                format!("{}", (bps * 2.0) as u32),
            ]
        };
        let x264 = ["-c:v", "libx264", "-preset", "veryfast", "-pix_fmt", "yuv420p"].map(String::from);
        match (s.quality, trimmed) {
            (Quality::Original, false) => a.extend(["-c:v", "copy"].map(String::from)),
            (Quality::Original, true) => {
                a.extend(x264);
                a.extend(["-crf", "18"].map(String::from));
            }
            (Quality::Light, _) => {
                a.extend(x264);
                a.extend(rate(self.target_bps(Quality::Light)));
            }
            (Quality::Minimal, _) => {
                a.extend(["-vf", "fps=30,scale=w='min(iw,1280)':h=-2"].map(String::from));
                a.extend(x264);
                a.extend(rate(self.target_bps(Quality::Minimal)));
            }
        }
        match audio_graph(&s.tracks) {
            None => a.push("-an".into()),
            Some(graph) => {
                a.extend(["-filter_complex".into(), graph, "-map".into(), "[a]".into()]);
                a.extend([
                    "-c:a".into(),
                    "aac".into(),
                    "-b:a".into(),
                    format!("{}", Self::audio_bps(s.quality)),
                ]);
            }
        }
        a.extend(["-movflags", "+faststart", "-progress", "pipe:1", "-nostats"].map(String::from));
        a.push(out.to_string_lossy().into_owned());
        a
    }
}

fn audio_graph(tracks: &[bool]) -> Option<String> {
    let chain = |i: usize| if i == MIC { DENOISE } else { "anull" };
    let on: Vec<usize> = (0..tracks.len()).filter(|&i| tracks[i]).collect();
    match on.as_slice() {
        [] => None,
        [i] => Some(format!("[0:a:{i}]{}[a]", chain(*i))),
        many => {
            let inputs: String = many.iter().map(|&i| format!("[0:a:{i}]{}[t{i}];", chain(i))).collect();
            let labels: String = many.iter().map(|i| format!("[t{i}]")).collect();
            Some(format!("{inputs}{labels}amix=inputs={}:normalize=0[a]", many.len()))
        }
    }
}

fn secs(t: f32) -> String {
    format!("{:.3}", t.max(0.0))
}

static GEN: AtomicU64 = AtomicU64::new(1);

pub fn next_id() -> u64 {
    GEN.fetch_add(1, Ordering::Relaxed)
}

fn ffmpeg(args: &[String], nice: bool) -> std::io::Result<Child> {
    let mut cmd = Command::new("ffmpeg");
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    unsafe {
        cmd.pre_exec(move || {
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
            if nice {
                libc::setpriority(libc::PRIO_PROCESS, 0, 10);
            }
            Ok(())
        });
    }
    cmd.spawn()
}

fn last_error(stderr: Option<std::process::ChildStderr>) -> String {
    let mut s = String::new();
    if let Some(mut e) = stderr {
        let _ = e.read_to_string(&mut s);
    }
    s.lines().last().unwrap_or_default().to_string()
}

pub struct Proc {
    pid: i32,
}

impl Proc {
    pub fn kill(&self) {
        unsafe {
            libc::kill(self.pid, libc::SIGKILL);
        }
    }
}

pub type FramePool = Arc<Mutex<Vec<Vec<u8>>>>;

pub struct PlayRequest<'a> {
    pub path: &'a Path,
    pub from: f32,
    pub to: f32,
    pub size: (u32, u32),
    pub tracks: &'a [bool],
    pub still: bool,
}

pub fn play(req: PlayRequest, id: u64, pool: FramePool, tx: Sender<Msg>) -> Result<Proc, String> {
    let (w, h) = req.size;
    let mut a: Vec<String> = ["-nostdin", "-v", "error"].map(String::from).to_vec();
    if !req.still {
        a.push("-re".into());
    }
    a.extend([
        "-ss".into(),
        secs(req.from),
        "-t".into(),
        secs(req.to - req.from),
        "-i".into(),
        req.path.to_string_lossy().into_owned(),
    ]);
    let audio = if req.still { None } else { audio_graph(req.tracks) };
    let mut graph = format!("[0:v:0]fps={PREVIEW_FPS},scale={w}:{h}:flags=bilinear,format=bgra[v]");
    if let Some(g) = &audio {
        graph.push(';');
        graph.push_str(g);
    }
    a.extend(["-filter_complex".into(), graph, "-map".into(), "[v]".into()]);
    if req.still {
        a.extend(["-frames:v", "1"].map(String::from));
    }
    a.extend(["-f", "rawvideo", "pipe:1"].map(String::from));
    if audio.is_some() {
        a.extend(["-map", "[a]", "-f", "pulse", "-buffer_duration", "40", "Ahri"].map(String::from));
    }

    let mut child = ffmpeg(&a, false).map_err(|e| format!("ffmpeg: {e}"))?;
    let pid = child.id() as i32;
    let mut stdout = child.stdout.take().ok_or("ffmpeg stdout")?;
    let stderr = child.stderr.take();
    let still = req.still;
    let len = w as usize * h as usize * 4;
    std::thread::spawn(move || {
        let mut start = None;
        for n in 0u32.. {
            let mut buf = pool.lock().ok().and_then(|mut p| p.pop()).unwrap_or_default();
            buf.resize(len, 0);
            if stdout.read_exact(&mut buf).is_err() {
                break;
            }
            if !still {
                let due = *start.get_or_insert_with(Instant::now) + Duration::from_secs_f32(n as f32 / PREVIEW_FPS);
                std::thread::sleep(due.saturating_duration_since(Instant::now()));
            }
            if tx.send(Msg::Media(Event::Frame { id, data: buf })).is_err() {
                return;
            }
        }
        let err = last_error(stderr);
        let status = child.wait();
        if !err.is_empty() && status.is_ok_and(|s| !s.success() && s.code().is_some()) {
            eprintln!("ahri: preview: {err}");
        }
        if !still {
            let _ = tx.send(Msg::Media(Event::Ended { id }));
        }
    });
    Ok(Proc { pid })
}

pub fn load_waves(path: &Path, tx: Sender<Msg>) {
    for track in 0..TRACKS {
        let path = path.to_path_buf();
        let tx = tx.clone();
        std::thread::spawn(move || {
            let peaks = wave(&path, track)
                .map_err(|e| eprintln!("ahri: wave {track}: {e}"))
                .ok();
            let _ = tx.send(Msg::Media(Event::Wave { track, peaks }));
        });
    }
}

pub fn probe(path: &Path, tx: Sender<Msg>) {
    let path = path.to_path_buf();
    std::thread::spawn(move || {
        let out = Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-select_streams",
                "v:0",
                "-show_entries",
                "stream=duration",
                "-of",
                "csv=p=0",
            ])
            .arg(&path)
            .stdin(Stdio::null())
            .output();
        let secs = out
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .and_then(|s| s.trim().parse::<f32>().ok());
        match secs {
            Some(secs) if secs > 0.0 => {
                let _ = tx.send(Msg::Media(Event::Duration { secs }));
            }
            _ => eprintln!("ahri: probe {}: no video duration", path.display()),
        }
    });
}

fn wave(path: &Path, track: usize) -> Result<Vec<f32>, String> {
    let args: Vec<String> = [
        "-nostdin",
        "-v",
        "error",
        "-i",
        &path.to_string_lossy(),
        "-map",
        &format!("0:a:{track}"),
        "-ac",
        "1",
        "-ar",
        &WAVE_RATE.to_string(),
        "-f",
        "s16le",
        "pipe:1",
    ]
    .map(String::from)
    .to_vec();
    let mut child = ffmpeg(&args, true).map_err(|e| e.to_string())?;
    let mut raw = Vec::new();
    child
        .stdout
        .take()
        .ok_or("stdout")?
        .read_to_end(&mut raw)
        .map_err(|e| e.to_string())?;
    let err = last_error(child.stderr.take());
    let status = child.wait().map_err(|e| e.to_string())?;
    if !status.success() || raw.is_empty() {
        return Err(err);
    }
    let samples: Vec<i16> = raw.as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes(*b)).collect();
    Ok(peaks(&samples, WAVE_BUCKETS))
}

pub fn peaks(samples: &[i16], buckets: usize) -> Vec<f32> {
    if samples.is_empty() {
        return vec![0.0; buckets];
    }
    (0..buckets)
        .map(|b| {
            let start = b * samples.len() / buckets;
            let end = ((b + 1) * samples.len() / buckets).max(start + 1).min(samples.len());
            let peak = samples[start..end].iter().map(|s| s.unsigned_abs()).max().unwrap_or(0);
            peak as f32 / 32768.0
        })
        .collect()
}

pub fn export(source: &Source, s: &Settings, out: &Path, id: u64, tx: Sender<Msg>) -> Result<Proc, String> {
    let args = source.export_args(s, out);
    let mut child = ffmpeg(&args, true).map_err(|e| format!("ffmpeg: {e}"))?;
    let pid = child.id() as i32;
    let stdout = child.stdout.take().ok_or("ffmpeg stdout")?;
    let stderr = child.stderr.take();
    let total = ((s.to - s.from).max(0.01) * 1_000_000.0) as f64;
    let out = out.to_path_buf();
    std::thread::spawn(move || {
        let mut last = 0.0f32;
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Some(us) = line.strip_prefix("out_time_us=").and_then(|v| v.parse::<f64>().ok()) {
                let done = (us / total).clamp(0.0, 1.0) as f32;
                if done - last >= 0.02 {
                    last = done;
                    let _ = tx.send(Msg::Media(Event::Progress { id, done }));
                }
            }
        }
        let err = last_error(stderr);
        let status = child.wait();
        let result = match status {
            Ok(st) if st.success() => out.metadata().map(|m| m.len()).map_err(|e| e.to_string()),
            Ok(st) => Err(format!("ffmpeg {st}: {err}")),
            Err(e) => Err(e.to_string()),
        };
        let _ = tx.send(Msg::Media(Event::Exported { id, result }));
    });
    Ok(Proc { pid })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source() -> Source {
        Source {
            path: PathBuf::from("/v/.ahri-raw-x.mp4"),
            duration: 10.0,
            bytes: 2_500_000,
        }
    }

    fn settings(quality: Quality, from: f32, to: f32, tracks: [bool; 2]) -> Settings {
        Settings {
            tracks: tracks.to_vec(),
            quality,
            from,
            to,
        }
    }

    fn args(s: &Settings) -> String {
        source().export_args(s, Path::new("/v/out.mp4")).join(" ")
    }

    #[test]
    fn untrimmed_original_copies_video() {
        let a = args(&settings(Quality::Original, 0.0, 10.0, [true, false]));
        assert!(a.contains("-c:v copy"));
        assert!(!a.contains("-ss "));
        assert!(a.contains("[0:a:0]anull[a]"));
    }

    #[test]
    fn trimmed_export_seeks_and_reencodes() {
        let a = args(&settings(Quality::Original, 1.5, 6.0, [true, true]));
        assert!(a.contains("-ss 1.500 -t 4.500 -i /v/.ahri-raw-x.mp4"));
        assert!(a.contains("-c:v libx264"));
        assert!(a.contains(&format!(
            "[0:a:0]anull[t0];[0:a:1]{DENOISE}[t1];[t0][t1]amix=inputs=2:normalize=0[a]"
        )));
    }

    #[test]
    fn no_tracks_drops_audio() {
        let a = args(&settings(Quality::Minimal, 0.0, 10.0, [false, false]));
        assert!(a.contains("-an"));
        assert!(!a.contains("-filter_complex"));
        assert!(a.contains("fps=30"));
    }

    #[test]
    fn lower_quality_is_smaller() {
        let src = source();
        let size = |q| src.estimate(&settings(q, 0.0, 10.0, [true, false]));
        assert!(size(Quality::Original) > size(Quality::Light));
        assert!(size(Quality::Light) > size(Quality::Minimal));
        let half = src.estimate(&settings(Quality::Light, 0.0, 5.0, [true, false]));
        assert_eq!(half, size(Quality::Light) / 2);
    }

    #[test]
    fn peaks_keep_loudest_sample_per_bucket() {
        let samples = [0, 100, -32768, 5, 16384, 0];
        assert_eq!(peaks(&samples, 3), vec![100.0 / 32768.0, 1.0, 0.5]);
        assert_eq!(peaks(&[], 2), vec![0.0, 0.0]);
    }
}
