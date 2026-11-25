use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use smithay_client_toolkit::reexports::calloop::channel::Sender;

use crate::Msg;

pub struct Recording {
    pid: i32,
    pub path: PathBuf,
    pub size: (u32, u32),
    pub started: std::time::Instant,
}

impl Recording {
    pub fn start(x: i32, y: i32, w: u32, h: u32, audio: &[&str], tx: Sender<Msg>) -> Result<Self, String> {
        let dir = dir();
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let name = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string();
        let path = dir.join(format!("{RAW_PREFIX}{name}.mp4"));

        let mut child = Command::new("gpu-screen-recorder")
            .args(["-w", &format!("{w}x{h}+{x}+{y}")])
            .args(["-f", "60", "-k", "h264", "-c", "mp4", "-cursor", "yes"])
            .args(audio.iter().flat_map(|a| ["-a", a]))
            .args(["-fallback-cpu-encoding", "yes", "-o"])
            .arg(&path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("gpu-screen-recorder: {e}"))?;

        let pid = child.id() as i32;
        let stderr = child.stderr.take().expect("stderr is piped");
        let out = path.clone();
        std::thread::spawn(move || {
            use std::io::BufRead;
            let last = std::io::BufReader::new(stderr)
                .lines()
                .map_while(Result::ok)
                .filter(|l| l.contains("error"))
                .last()
                .unwrap_or_default();
            let status = child.wait();
            let ok = status.as_ref().is_ok_and(|s| s.success()) && out.metadata().is_ok_and(|m| m.len() > 0);
            let result = if ok {
                Ok(out)
            } else {
                Err(format!("gpu-screen-recorder {status:?} {last}"))
            };
            let _ = tx.send(Msg::RecordingDone(result));
        });

        Ok(Self {
            pid,
            path,
            size: (w, h),
            started: std::time::Instant::now(),
        })
    }

    pub fn stop(&self) {
        unsafe {
            libc::kill(self.pid, libc::SIGINT);
        }
    }
}

const RAW_PREFIX: &str = ".ahri-raw-";
const EXPORT_PREFIX: &str = ".ahri-export-";

pub fn dir() -> PathBuf {
    videos_dir().join("Ahri")
}

pub fn final_path(raw: &Path) -> PathBuf {
    let name = raw
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stem = name.strip_prefix(RAW_PREFIX).unwrap_or(&name);
    raw.with_file_name(format!("ahri_{stem}"))
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
            let to = final_path(&path);
            if to.exists() {
                let _ = std::fs::remove_file(&path);
                continue;
            }
            match std::fs::rename(&path, &to) {
                Ok(()) => eprintln!("ahri: recovered {}", to.display()),
                Err(e) => eprintln!("ahri: recover {}: {e}", path.display()),
            }
        }
    }
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
    fn raw_names_map_to_final_and_export_names() {
        let raw = Path::new("/v/Ahri/.ahri-raw-2026-11-25_12-00-00.mp4");
        assert_eq!(final_path(raw), Path::new("/v/Ahri/ahri_2026-11-25_12-00-00.mp4"));
        assert_eq!(export_path(raw, 7), Path::new("/v/Ahri/.ahri-export-7.mp4"));
    }
}
