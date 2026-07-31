use std::{
    collections::BTreeMap,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use hachimi_protocol::{
    InstalledContribution, PluginContribution, PluginContributionKind, PluginContributionSurface,
    PluginId, PluginUiBridgeMethod,
};
use parking_lot::RwLock;
use sha2::{Digest, Sha256};
use tauri::Manager;

use crate::DesktopState;

const UI_CSP: &str = "default-src 'none'; img-src data: hachimi-plugin-asset: http://hachimi-plugin-asset.localhost; style-src 'self'; script-src 'self'; connect-src hachimi-plugin-asset: http://hachimi-plugin-asset.localhost; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'; navigate-to 'none'";
const UI_BOOTSTRAP_PATH: &str = "__hachimi_host_bootstrap__.js";
const UI_BOOTSTRAP: &str = r#"(() => {
  const denied = () => { throw new Error("plugin_tauri_ipc_denied"); };
  const internals = window.__TAURI_INTERNALS__;
  if (internals && typeof internals === "object") {
    for (const key of Reflect.ownKeys(internals)) {
      const descriptor = Object.getOwnPropertyDescriptor(internals, key);
      const replacement = typeof descriptor?.value === "function" ? denied : undefined;
      try {
        if (descriptor?.writable) {
          internals[key] = replacement;
        } else if (descriptor?.configurable) {
          Object.defineProperty(internals, key, {
            value: replacement,
            writable: false,
            enumerable: false,
            configurable: false,
          });
        }
      } catch {}
    }
    try { Object.freeze(internals); } catch {}
  }
  try { Reflect.deleteProperty(window, "__TAURI__"); } catch {}
  try {
    Object.defineProperty(window, "__TAURI__", {
      value: undefined,
      writable: false,
      enumerable: false,
      configurable: false,
    });
  } catch {
    try { window.__TAURI__ = undefined; } catch {}
  }
})();"#;

#[derive(Debug, Clone)]
struct ServedFile {
    path: PathBuf,
    sha256: String,
    mime: &'static str,
}

#[derive(Debug, Clone)]
struct ContributionSurfaceEntry {
    surface: PluginContributionSurface,
    files: BTreeMap<String, ServedFile>,
    entry_path: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct PluginSurfaceRegistry {
    entries: Arc<RwLock<BTreeMap<(String, String), ContributionSurfaceEntry>>>,
}

impl PluginSurfaceRegistry {
    pub(super) fn reconcile(
        &self,
        plugin_id: &PluginId,
        contribution: &PluginContribution,
        runtime: &InstalledContribution,
        target: &Path,
        enabled: bool,
    ) -> Result<(), &'static str> {
        let key = (plugin_id.as_str().to_owned(), contribution.id.clone());
        if !enabled
            || !matches!(
                contribution.kind,
                PluginContributionKind::Asset | PluginContributionKind::CustomUi
            )
        {
            self.entries.write().remove(&key);
            return Ok(());
        }
        let (files, entry_path) = collect_surface_files(contribution.kind, target)?;
        let route = format!(
            "{}/{}/{}/",
            plugin_id.as_str(),
            contribution.id,
            runtime.runtime_revision
        );
        let surface = PluginContributionSurface {
            plugin_id: plugin_id.clone(),
            contribution_id: contribution.id.clone(),
            kind: contribution.kind,
            runtime_revision: runtime.runtime_revision.clone(),
            runtime_state: runtime.state,
            diagnostic: runtime.diagnostic.clone(),
            last_result_code: None,
            entry_url: entry_path
                .as_ref()
                .map(|entry| surface_url("hachimi-plugin-ui", &format!("{route}{entry}"))),
            asset_base_url: (contribution.kind == PluginContributionKind::Asset)
                .then(|| surface_url("hachimi-plugin-asset", &route)),
            allowed_bridge_methods: if contribution.kind == PluginContributionKind::CustomUi {
                vec![
                    PluginUiBridgeMethod::GetContext,
                    PluginUiBridgeMethod::ResolveAssetUrl,
                    PluginUiBridgeMethod::Close,
                ]
            } else {
                Vec::new()
            },
        };
        self.entries.write().insert(
            key,
            ContributionSurfaceEntry {
                surface,
                files,
                entry_path,
            },
        );
        Ok(())
    }

    pub(super) fn remove_plugin(&self, plugin_id: &PluginId) {
        self.entries
            .write()
            .retain(|(installed_id, _), _| installed_id != plugin_id.as_str());
    }

    pub(super) fn surface(
        &self,
        plugin_id: &PluginId,
        contribution_id: &str,
    ) -> Option<PluginContributionSurface> {
        self.entries
            .read()
            .get(&(plugin_id.as_str().to_owned(), contribution_id.to_owned()))
            .map(|entry| entry.surface.clone())
    }

    fn resolve(&self, route: &SurfaceRoute, kind: PluginContributionKind) -> Option<ServedFile> {
        let entries = self.entries.read();
        let entry = entries.get(&(route.plugin_id.clone(), route.contribution_id.clone()))?;
        if entry.surface.kind != kind || entry.surface.runtime_revision != route.runtime_revision {
            return None;
        }
        let relative = if route.relative_path.is_empty() {
            entry.entry_path.as_deref()?
        } else {
            &route.relative_path
        };
        entry.files.get(relative).cloned()
    }
}

fn surface_url(scheme: &str, route: &str) -> String {
    if cfg!(windows) {
        format!("http://{scheme}.localhost/{route}")
    } else {
        format!("{scheme}://localhost/{route}")
    }
}

pub(super) fn asset_protocol(
    context: tauri::UriSchemeContext<'_, tauri::Wry>,
    request: tauri::http::Request<Vec<u8>>,
) -> tauri::http::Response<Vec<u8>> {
    serve_protocol(context, request, PluginContributionKind::Asset)
}

pub(super) fn ui_protocol(
    context: tauri::UriSchemeContext<'_, tauri::Wry>,
    request: tauri::http::Request<Vec<u8>>,
) -> tauri::http::Response<Vec<u8>> {
    serve_protocol(context, request, PluginContributionKind::CustomUi)
}

fn serve_protocol(
    context: tauri::UriSchemeContext<'_, tauri::Wry>,
    request: tauri::http::Request<Vec<u8>>,
    kind: PluginContributionKind,
) -> tauri::http::Response<Vec<u8>> {
    if context.webview_label() != "workbench" || request.method() != tauri::http::Method::GET {
        return response(
            tauri::http::StatusCode::FORBIDDEN,
            "text/plain",
            b"forbidden".to_vec(),
            kind,
        );
    }
    let Some(route) = parse_route(request.uri().path()) else {
        return response(
            tauri::http::StatusCode::BAD_REQUEST,
            "text/plain",
            b"invalid route".to_vec(),
            kind,
        );
    };
    let Some(state) = context.app_handle().try_state::<DesktopState>() else {
        return response(
            tauri::http::StatusCode::SERVICE_UNAVAILABLE,
            "text/plain",
            b"runtime unavailable".to_vec(),
            kind,
        );
    };
    if kind == PluginContributionKind::CustomUi && route.relative_path == UI_BOOTSTRAP_PATH {
        if state
            .plugin_surfaces
            .surface(
                &PluginId::from(route.plugin_id.as_str()),
                &route.contribution_id,
            )
            .is_none_or(|surface| surface.runtime_revision != route.runtime_revision)
        {
            return response(
                tauri::http::StatusCode::NOT_FOUND,
                "text/plain",
                b"surface unavailable".to_vec(),
                kind,
            );
        }
        return response(
            tauri::http::StatusCode::OK,
            "text/javascript; charset=utf-8",
            UI_BOOTSTRAP.as_bytes().to_vec(),
            kind,
        );
    }
    let Some(file) = state.plugin_surfaces.resolve(&route, kind) else {
        return response(
            tauri::http::StatusCode::NOT_FOUND,
            "text/plain",
            b"surface unavailable".to_vec(),
            kind,
        );
    };
    let Ok(canonical) = file.path.canonicalize() else {
        return response(
            tauri::http::StatusCode::NOT_FOUND,
            "text/plain",
            b"surface drift".to_vec(),
            kind,
        );
    };
    if canonical != file.path {
        return response(
            tauri::http::StatusCode::FORBIDDEN,
            "text/plain",
            b"surface drift".to_vec(),
            kind,
        );
    }
    let Ok(bytes) = std::fs::read(&canonical) else {
        return response(
            tauri::http::StatusCode::NOT_FOUND,
            "text/plain",
            b"surface missing".to_vec(),
            kind,
        );
    };
    if digest(&bytes) != file.sha256 {
        return response(
            tauri::http::StatusCode::CONFLICT,
            "text/plain",
            b"surface hash drift".to_vec(),
            kind,
        );
    }
    let bytes = if kind == PluginContributionKind::CustomUi && file.mime.starts_with("text/html") {
        harden_custom_ui_html(bytes)
    } else {
        bytes
    };
    response(tauri::http::StatusCode::OK, file.mime, bytes, kind)
}

fn harden_custom_ui_html(bytes: Vec<u8>) -> Vec<u8> {
    const PREFIX: &[u8] = b"<script src=\"__hachimi_host_bootstrap__.js\"></script>\n";
    let mut hardened = Vec::with_capacity(PREFIX.len() + bytes.len());
    hardened.extend_from_slice(PREFIX);
    hardened.extend_from_slice(&bytes);
    hardened
}

fn response(
    status: tauri::http::StatusCode,
    mime: &'static str,
    body: Vec<u8>,
    kind: PluginContributionKind,
) -> tauri::http::Response<Vec<u8>> {
    let mut builder = tauri::http::Response::builder()
        .status(status)
        .header(tauri::http::header::CONTENT_TYPE, mime)
        .header("X-Content-Type-Options", "nosniff")
        .header(tauri::http::header::CACHE_CONTROL, "no-store")
        .header(tauri::http::header::ACCESS_CONTROL_ALLOW_ORIGIN, "*");
    if kind == PluginContributionKind::CustomUi {
        builder = builder.header("Content-Security-Policy", UI_CSP);
    }
    builder.body(body).expect("static plugin response headers")
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SurfaceRoute {
    plugin_id: String,
    contribution_id: String,
    runtime_revision: String,
    relative_path: String,
}

fn parse_route(path: &str) -> Option<SurfaceRoute> {
    let mut segments = path.trim_start_matches('/').split('/');
    let plugin_id = decode_segment(segments.next()?)?;
    let contribution_id = decode_segment(segments.next()?)?;
    let runtime_revision = decode_segment(segments.next()?)?;
    if plugin_id.is_empty() || contribution_id.is_empty() || runtime_revision.len() != 64 {
        return None;
    }
    let relative_path = segments
        .map(decode_segment)
        .collect::<Option<Vec<_>>>()?
        .join("/");
    Some(SurfaceRoute {
        plugin_id,
        contribution_id,
        runtime_revision,
        relative_path,
    })
}

fn decode_segment(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let high = hex(*bytes.get(index + 1)?)?;
            let low = hex(*bytes.get(index + 2)?)?;
            decoded.push((high << 4) | low);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    let value = String::from_utf8(decoded).ok()?;
    (!value.is_empty() && value != "." && value != ".." && !value.contains(['/', '\\', '\0']))
        .then_some(value)
}

const fn hex(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn collect_surface_files(
    kind: PluginContributionKind,
    target: &Path,
) -> Result<(BTreeMap<String, ServedFile>, Option<String>), &'static str> {
    let root = target
        .canonicalize()
        .map_err(|_| "plugin_surface_root_missing")?;
    if !root.is_dir() {
        return Err("plugin_surface_root_not_directory");
    }
    let mut files = BTreeMap::new();
    collect_directory(kind, &root, &root, &mut files)?;
    if files.len() > 2_000 {
        return Err("plugin_surface_file_limit");
    }
    let entry = if kind == PluginContributionKind::CustomUi {
        Some(
            files
                .contains_key("index.html")
                .then(|| "index.html".to_owned())
                .ok_or("plugin_custom_ui_entry_missing")?,
        )
    } else {
        None
    };
    Ok((files, entry))
}

fn collect_directory(
    kind: PluginContributionKind,
    root: &Path,
    current: &Path,
    files: &mut BTreeMap<String, ServedFile>,
) -> Result<(), &'static str> {
    for entry in std::fs::read_dir(current).map_err(|_| "plugin_surface_read_failed")? {
        let entry = entry.map_err(|_| "plugin_surface_read_failed")?;
        let metadata = entry
            .file_type()
            .map_err(|_| "plugin_surface_metadata_failed")?;
        if metadata.is_symlink() {
            return Err("plugin_surface_symlink_denied");
        }
        let path = entry.path();
        if metadata.is_dir() {
            collect_directory(kind, root, &path, files)?;
            continue;
        }
        if !metadata.is_file() {
            return Err("plugin_surface_special_file_denied");
        }
        let canonical = path
            .canonicalize()
            .map_err(|_| "plugin_surface_canonicalize_failed")?;
        if !canonical.starts_with(root) {
            return Err("plugin_surface_path_escape");
        }
        let relative = canonical
            .strip_prefix(root)
            .map_err(|_| "plugin_surface_path_escape")?;
        if relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err("plugin_surface_path_escape");
        }
        let relative = relative.to_string_lossy().replace('\\', "/");
        let mime = allowed_mime(kind, &canonical).ok_or("plugin_surface_mime_denied")?;
        let bytes = std::fs::read(&canonical).map_err(|_| "plugin_surface_read_failed")?;
        files.insert(
            relative,
            ServedFile {
                path: canonical,
                sha256: digest(&bytes),
                mime,
            },
        );
    }
    Ok(())
}

fn allowed_mime(kind: PluginContributionKind, path: &Path) -> Option<&'static str> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    match (kind, extension.as_str()) {
        (PluginContributionKind::CustomUi, "html") => Some("text/html; charset=utf-8"),
        (PluginContributionKind::CustomUi, "js") => Some("text/javascript; charset=utf-8"),
        (PluginContributionKind::CustomUi, "css") => Some("text/css; charset=utf-8"),
        (PluginContributionKind::Asset, "txt") => Some("text/plain; charset=utf-8"),
        (PluginContributionKind::Asset, "json") => Some("application/json"),
        (PluginContributionKind::Asset, "css") => Some("text/css; charset=utf-8"),
        (PluginContributionKind::Asset, "png") => Some("image/png"),
        (PluginContributionKind::Asset, "jpg" | "jpeg") => Some("image/jpeg"),
        (PluginContributionKind::Asset, "gif") => Some("image/gif"),
        (PluginContributionKind::Asset, "webp") => Some("image/webp"),
        (PluginContributionKind::Asset, "woff") => Some("font/woff"),
        (PluginContributionKind::Asset, "woff2") => Some("font/woff2"),
        _ => None,
    }
}

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_rejects_encoded_traversal() {
        assert!(parse_route("/sample/ui/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/%2e%2e/x").is_none());
        assert!(
            parse_route(
                "/sample/ui/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/%2fetc"
            )
            .is_none()
        );
    }

    #[test]
    fn executable_asset_mime_is_denied() {
        assert_eq!(
            allowed_mime(PluginContributionKind::Asset, Path::new("fixture.exe")),
            None
        );
        assert_eq!(
            allowed_mime(PluginContributionKind::Asset, Path::new("fixture.svg")),
            None
        );
    }

    #[test]
    fn surface_urls_use_the_platform_custom_protocol_origin() {
        let value = surface_url("hachimi-plugin-ui", "sample/dashboard/revision/index.html");
        if cfg!(windows) {
            assert_eq!(
                value,
                "http://hachimi-plugin-ui.localhost/sample/dashboard/revision/index.html"
            );
        } else {
            assert_eq!(
                value,
                "hachimi-plugin-ui://localhost/sample/dashboard/revision/index.html"
            );
        }
    }

    #[test]
    fn custom_ui_html_runs_the_host_isolation_bootstrap_first() {
        let hardened =
            harden_custom_ui_html(b"<!doctype html><script src=\"app.js\"></script>".to_vec());
        let text = String::from_utf8(hardened).expect("UTF-8 fixture");
        assert!(text.starts_with(
            "<script src=\"__hachimi_host_bootstrap__.js\"></script>\n<!doctype html>"
        ));
        assert!(UI_BOOTSTRAP.contains("__TAURI_INTERNALS__"));
        assert!(UI_BOOTSTRAP.contains("plugin_tauri_ipc_denied"));
        assert!(UI_BOOTSTRAP.contains("Object.freeze(internals)"));
    }
}
