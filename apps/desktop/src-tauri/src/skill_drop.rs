use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use serde::Serialize;
use tauri::{DragDropEvent, Emitter, Manager, Runtime, Webview, WebviewEvent};
use uuid::Uuid;

use super::DesktopState;

const SKILL_DROP_TOKEN_TTL: Duration = Duration::from_secs(2 * 60);
const MAX_PENDING_SKILL_DROPS: usize = 64;
const MAX_SKILL_DROP_PATHS: usize = 513;
const SKILL_NATIVE_DRAG_EVENT: &str = "skills:native-drag";

#[derive(Debug)]
pub(super) struct PendingSkillDrop {
    pub(super) owner: String,
    pub(super) paths: Vec<PathBuf>,
    pub(super) expires_at: Instant,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SkillNativeDragEvent {
    kind: &'static str,
    token: Option<String>,
    x: Option<f64>,
    y: Option<f64>,
    file_names: Vec<String>,
}

pub(super) fn handle_skill_drag_event<R: Runtime>(webview: &Webview<R>, event: &WebviewEvent) {
    if webview.label() != "workbench" {
        return;
    }
    let payload = match event {
        WebviewEvent::DragDrop(DragDropEvent::Enter { paths, position }) => SkillNativeDragEvent {
            kind: "enter",
            token: None,
            x: Some(position.x),
            y: Some(position.y),
            file_names: skill_drop_file_names(paths),
        },
        WebviewEvent::DragDrop(DragDropEvent::Over { position }) => SkillNativeDragEvent {
            kind: "over",
            token: None,
            x: Some(position.x),
            y: Some(position.y),
            file_names: Vec::new(),
        },
        WebviewEvent::DragDrop(DragDropEvent::Drop { paths, position }) => {
            let Some(state) = webview.app_handle().try_state::<DesktopState>() else {
                return;
            };
            let now = Instant::now();
            let token = Uuid::now_v7().to_string();
            let mut pending = state.pending_skill_drops.lock();
            pending.retain(|_, drop| drop.expires_at > now);
            while pending.len() >= MAX_PENDING_SKILL_DROPS {
                let Some(oldest_key) = pending.keys().next().cloned() else {
                    break;
                };
                pending.remove(&oldest_key);
            }
            pending.insert(
                token.clone(),
                PendingSkillDrop {
                    owner: webview.label().to_owned(),
                    paths: paths.iter().take(MAX_SKILL_DROP_PATHS).cloned().collect(),
                    expires_at: now + SKILL_DROP_TOKEN_TTL,
                },
            );
            SkillNativeDragEvent {
                kind: "drop",
                token: Some(token),
                x: Some(position.x),
                y: Some(position.y),
                file_names: skill_drop_file_names(paths),
            }
        }
        WebviewEvent::DragDrop(DragDropEvent::Leave) => SkillNativeDragEvent {
            kind: "leave",
            token: None,
            x: None,
            y: None,
            file_names: Vec::new(),
        },
        _ => return,
    };
    let _ = webview.emit(SKILL_NATIVE_DRAG_EVENT, payload);
}

fn skill_drop_file_names(paths: &[PathBuf]) -> Vec<String> {
    paths
        .iter()
        .filter_map(|path| Path::file_name(path))
        .filter_map(|name| name.to_str())
        .take(512)
        .map(str::to_owned)
        .collect()
}
