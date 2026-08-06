use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

static TRACE_PATH: OnceLock<Option<PathBuf>> = OnceLock::new();

pub(crate) fn record(stage: &str) {
    let Some(path) = TRACE_PATH
        .get_or_init(|| {
            std::env::args().find_map(|argument| {
                argument
                    .strip_prefix("--hachimi-profile-dir=")
                    .map(PathBuf::from)
                    .map(|directory| directory.join("cef-bootstrap.log"))
            })
        })
        .as_ref()
    else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) else {
        return;
    };
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    let _ = writeln!(file, "{timestamp} pid={} {stage}", std::process::id());
    let _ = file.flush();
}
