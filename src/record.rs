use std::path::PathBuf;
use std::process::{Command, Stdio};

use smithay_client_toolkit::reexports::calloop::channel::Sender;

use crate::Msg;

pub struct Recording {
    pid: i32,
    pub path: PathBuf,
}

impl Recording {
    pub fn start(x: i32, y: i32, w: u32, h: u32, tx: Sender<Msg>) -> Result<Self, String> {
        let dir = videos_dir().join("Ahri");
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let name = chrono::Local::now().format("ahri_%Y-%m-%d_%H-%M-%S.mp4").to_string();
        let path = dir.join(name);

        let mut child = Command::new("gpu-screen-recorder")
            .args(["-w", &format!("{w}x{h}+{x}+{y}")])
            .args(["-f", "60", "-k", "h264", "-c", "mp4", "-cursor", "yes"])
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

        Ok(Self { pid, path })
    }

    pub fn stop(&self) {
        unsafe {
            libc::kill(self.pid, libc::SIGINT);
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
