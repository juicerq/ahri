use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::time::Instant;

use smithay_client_toolkit::reexports::calloop::channel::Sender;

use crate::Msg;

pub struct Recording {
    pid: Arc<AtomicI32>,
    stopped: Arc<AtomicBool>,
    pub path: PathBuf,
    pub size: (u32, u32),
    pub started: Instant,
}

impl Recording {
    pub fn start(x: i32, y: i32, w: u32, h: u32, audio: Vec<&'static str>, tx: Sender<Msg>) -> Result<Self, String> {
        let dir = dir();
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let name = unused(&dir, &chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string());
        let path = dir.join(format!("{RAW_PREFIX}{name}.mp4"));
        let region = format!("{w}x{h}+{x}+{y}");

        let child = spawn(&region, &audio, &path).map_err(|e| format!("gpu-screen-recorder: {e}"))?;
        let pid = Arc::new(AtomicI32::new(child.id() as i32));
        let stopped = Arc::new(AtomicBool::new(false));
        let (out, thread_pid, thread_stopped) = (path.clone(), pid.clone(), stopped.clone());
        std::thread::spawn(move || {
            let (mut child, mut audio) = (child, audio);
            loop {
                let last = last_error(&mut child);
                let status = child.wait();
                let written = out.metadata().is_ok_and(|m| m.len() > 0);
                let ok = status.as_ref().is_ok_and(|s| s.success()) && written;
                let mut error = format!("gpu-screen-recorder {status:?} {last}");
                if !ok && !written && !audio.is_empty() && !thread_stopped.load(Ordering::SeqCst) {
                    eprintln!("ahri: recorder failed with audio ({last}), retrying without audio");
                    audio.clear();
                    match spawn(&region, &audio, &out) {
                        Ok(next) => {
                            thread_pid.store(next.id() as i32, Ordering::SeqCst);
                            child = next;
                            if thread_stopped.load(Ordering::SeqCst) {
                                interrupt(&thread_pid);
                            }
                            continue;
                        }
                        Err(e) => error = format!("gpu-screen-recorder: {e}"),
                    }
                }
                let result = if ok { Ok(out) } else { Err(error) };
                let _ = tx.send(Msg::RecordingDone(result));
                return;
            }
        });

        Ok(Self {
            pid,
            stopped,
            path,
            size: (w, h),
            started: Instant::now(),
        })
    }

    pub fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        interrupt(&self.pid);
    }
}

fn spawn(region: &str, audio: &[&str], path: &Path) -> std::io::Result<Child> {
    Command::new("gpu-screen-recorder")
        .args(["-w", region])
        .args(["-f", &crate::media::RECORD_FPS.to_string()])
        .args(["-k", "h264", "-c", "mp4", "-cursor", "yes"])
        .args(audio.iter().flat_map(|a| ["-a", a]))
        .args(["-fallback-cpu-encoding", "yes", "-o"])
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
}

fn last_error(child: &mut Child) -> String {
    use std::io::BufRead;
    let Some(stderr) = child.stderr.take() else {
        return String::new();
    };
    std::io::BufReader::new(stderr)
        .lines()
        .map_while(Result::ok)
        .filter(|l| l.contains("error"))
        .last()
        .unwrap_or_default()
}

fn interrupt(pid: &AtomicI32) {
    unsafe {
        libc::kill(pid.load(Ordering::SeqCst), libc::SIGINT);
    }
}

const RAW_PREFIX: &str = ".ahri-raw-";
const EXPORT_PREFIX: &str = ".ahri-export-";

pub fn dir() -> PathBuf {
    videos_dir().join("Ahri")
}

pub fn publish(src: &Path, raw: &Path) -> std::io::Result<PathBuf> {
    let name = raw
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stem = name.trim_start_matches(RAW_PREFIX).trim_end_matches(".mp4");
    for n in 1.. {
        let dst = raw.with_file_name(if n == 1 {
            format!("ahri_{stem}.mp4")
        } else {
            format!("ahri_{stem}-{n}.mp4")
        });
        match move_new(src, &dst) {
            Ok(()) => return Ok(dst),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    unreachable!()
}

fn move_new(src: &Path, dst: &Path) -> std::io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let c = |p: &Path| std::ffi::CString::new(p.as_os_str().as_bytes()).map_err(std::io::Error::other);
    let (from, to) = (c(src)?, c(dst)?);
    let moved = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            from.as_ptr(),
            libc::AT_FDCWD,
            to.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if moved == 0 {
        return Ok(());
    }
    let err = std::io::Error::last_os_error();
    if err.raw_os_error() != Some(libc::EINVAL) {
        return Err(err);
    }
    std::fs::hard_link(src, dst)?;
    std::fs::remove_file(src)
}

pub fn export_path(raw: &Path, id: u64) -> PathBuf {
    raw.with_file_name(format!("{EXPORT_PREFIX}{id}.mp4"))
}

pub fn audio_sources() -> Vec<&'static str> {
    let listed = Command::new("gpu-screen-recorder")
        .arg("--list-audio-devices")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    let has = |name: &str| listed.lines().any(|l| l.split('|').next() == Some(name));
    match (has("default_output"), has("default_input")) {
        (true, true) => vec!["default_output", "default_input"],
        (true, false) => vec!["default_output"],
        _ => Vec::new(),
    }
}

pub fn recover() {
    let Ok(entries) = std::fs::read_dir(dir()) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with(EXPORT_PREFIX) {
            let _ = std::fs::remove_file(&path);
        } else if name.starts_with(RAW_PREFIX) {
            match publish(&path, &path) {
                Ok(to) => eprintln!("ahri: recovered {}", to.display()),
                Err(e) => eprintln!("ahri: recover {}: {e}", path.display()),
            }
        }
    }
}

fn unused(dir: &Path, stem: &str) -> String {
    (1..)
        .map(|n| {
            if n == 1 {
                stem.to_string()
            } else {
                format!("{stem}-{n}")
            }
        })
        .find(|s| !dir.join(format!("{RAW_PREFIX}{s}.mp4")).exists() && !dir.join(format!("ahri_{s}.mp4")).exists())
        .unwrap_or_default()
}

fn videos_dir() -> PathBuf {
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"));
    let dirs = std::fs::read_to_string(config.join("user-dirs.dirs")).unwrap_or_default();
    dirs.lines()
        .find_map(|l| l.strip_prefix("XDG_VIDEOS_DIR="))
        .map(|v| v.trim_matches('"').replace("$HOME", &home.to_string_lossy()))
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join("Videos"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publish_never_replaces_an_existing_video() {
        let dir = std::env::temp_dir().join(format!("ahri-publish-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let raw = dir.join(".ahri-raw-t.mp4");
        let export = export_path(&raw, 7);
        assert_eq!(export, dir.join(".ahri-export-7.mp4"));
        std::fs::write(dir.join("ahri_t.mp4"), b"old").unwrap();
        std::fs::write(&export, b"new").unwrap();
        let out = publish(&export, &raw).unwrap();
        assert_eq!(out, dir.join("ahri_t-2.mp4"));
        assert_eq!(std::fs::read(dir.join("ahri_t.mp4")).unwrap(), b"old");
        assert_eq!(std::fs::read(&out).unwrap(), b"new");
        assert!(!export.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn unused_skips_names_taken_by_raw_or_final_files() {
        let dir = std::env::temp_dir().join(format!("ahri-unused-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(unused(&dir, "t"), "t");
        std::fs::write(dir.join("ahri_t.mp4"), b"").unwrap();
        std::fs::write(dir.join(".ahri-raw-t-2.mp4"), b"").unwrap();
        assert_eq!(unused(&dir, "t"), "t-3");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
