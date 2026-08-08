use std::{fs, path::PathBuf};

use hachimi_protocol::{
    ConnectorActionDescriptor, ConnectorActionEffect, ConnectorRevision, ConnectorRuntimeKind,
    InstalledPlugin,
};
use serde_json::{Value, json};

use crate::{ExtensionHostError, safe_relative_path, value_hash};

#[derive(Debug, Clone)]
pub(crate) struct LoadedConnectorDescriptor {
    pub(crate) host_identity: String,
    pub(crate) runtime_kind: ConnectorRuntimeKind,
    pub(crate) entrypoint: Option<PathBuf>,
    pub(crate) args: Vec<String>,
    pub(crate) actions: Vec<ConnectorActionDescriptor>,
    pub(crate) revision: ConnectorRevision,
}

pub(crate) fn connector_descriptor(
    plugin: &InstalledPlugin,
    connector_id: &str,
) -> Result<LoadedConnectorDescriptor, ExtensionHostError> {
    let contribution = plugin
        .manifest
        .contributions
        .iter()
        .find(|contribution| {
            contribution.kind == hachimi_protocol::PluginContributionKind::Connector
                && contribution.id == connector_id
        })
        .ok_or(ExtensionHostError::ContributionDrift)?;
    let relative = safe_relative_path(&contribution.relative_path)?;
    let root = PathBuf::from(&plugin.root_path);
    let path = root.join(relative);
    let canonical_root = root
        .canonicalize()
        .map_err(|_| ExtensionHostError::ContributionDrift)?;
    let canonical_path = path
        .canonicalize()
        .map_err(|_| ExtensionHostError::ContributionDrift)?;
    if !canonical_path.starts_with(&canonical_root) || !canonical_path.is_file() {
        return Err(ExtensionHostError::ContributionDrift);
    }
    let descriptor: Value = serde_json::from_slice(&fs::read(canonical_path)?)?;
    let host_identity = descriptor
        .get("hostIdentity")
        .and_then(Value::as_str)
        .filter(|identity| !identity.trim().is_empty())
        .ok_or_else(|| {
            ExtensionHostError::InvalidManifest("connector hostIdentity missing".into())
        })?;
    let actions = descriptor
        .get("actions")
        .and_then(Value::as_array)
        .filter(|actions| !actions.is_empty())
        .ok_or_else(|| ExtensionHostError::InvalidManifest("connector actions missing".into()))?;
    let mut actions = actions
        .iter()
        .map(|action| {
            let action = action.as_object().ok_or_else(|| {
                ExtensionHostError::InvalidManifest("connector action is invalid".into())
            })?;
            let name = action
                .get("name")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty() && value.len() <= 128)
                .ok_or_else(|| {
                    ExtensionHostError::InvalidManifest("connector action name is invalid".into())
                })?;
            let effect = match action.get("effect").and_then(Value::as_str) {
                Some("read_only") => ConnectorActionEffect::ReadOnly,
                Some("external_side_effect") => ConnectorActionEffect::ExternalSideEffect,
                _ => {
                    return Err(ExtensionHostError::InvalidManifest(
                        "connector action effect is invalid".into(),
                    ));
                }
            };
            Ok(ConnectorActionDescriptor {
                name: name.to_owned(),
                effect,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    actions.sort_by(|left, right| left.name.cmp(&right.name));
    if actions.windows(2).any(|pair| pair[0].name == pair[1].name) {
        return Err(ExtensionHostError::InvalidManifest(
            "connector action names must be unique".into(),
        ));
    }
    let runtime_kind = match descriptor.get("transport").and_then(Value::as_str) {
        Some("local")
            if descriptor.get("externalNetwork").and_then(Value::as_bool) == Some(false) =>
        {
            ConnectorRuntimeKind::Builtin
        }
        Some("stdio_json_rpc") => ConnectorRuntimeKind::SandboxedStdioJsonRpc,
        _ => {
            return Err(ExtensionHostError::InvalidManifest(
                "connector transport is invalid".into(),
            ));
        }
    };
    let (entrypoint, args) = if runtime_kind == ConnectorRuntimeKind::SandboxedStdioJsonRpc {
        let expected_identity = format!(
            "hachimi.plugin.{}.{}.sidecar.v1",
            plugin.manifest.id, connector_id
        );
        if host_identity != expected_identity {
            return Err(ExtensionHostError::InvalidManifest(
                "sidecar connector hostIdentity is not namespaced to the plugin".into(),
            ));
        }
        let entrypoint = descriptor
            .get("entrypoint")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ExtensionHostError::InvalidManifest("connector entrypoint missing".into())
            })?;
        let entrypoint = root
            .join(safe_relative_path(entrypoint)?)
            .canonicalize()
            .map_err(|_| ExtensionHostError::ContributionDrift)?;
        if !entrypoint.starts_with(&canonical_root) || !entrypoint.is_file() {
            return Err(ExtensionHostError::ContributionEscape);
        }
        let args = descriptor
            .get("args")
            .and_then(Value::as_array)
            .map(|args| {
                args.iter()
                    .map(|arg| {
                        arg.as_str()
                            .filter(|arg| arg.len() <= 4_096 && !arg.contains('\0'))
                            .map(str::to_owned)
                            .ok_or_else(|| {
                                ExtensionHostError::InvalidManifest(
                                    "connector sidecar argument is invalid".into(),
                                )
                            })
                    })
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()?
            .unwrap_or_default();
        (Some(entrypoint), args)
    } else {
        (None, Vec::new())
    };
    let schema = descriptor.get("schema").cloned().unwrap_or_else(|| {
        json!({
            "record": { "id": "string", "data": "object", "revision": "integer" },
            "webhook": "metadata_only",
            "poll": "deterministic"
        })
    });
    let revision = ConnectorRevision {
        host_identity_hash: value_hash(&json!({
            "runtime": runtime_kind,
            "declared": host_identity,
        }))?,
        schema_hash: value_hash(&schema)?,
        action_hash: value_hash(&serde_json::to_value(&actions)?)?,
    };
    Ok(LoadedConnectorDescriptor {
        host_identity: host_identity.to_owned(),
        runtime_kind,
        entrypoint,
        args,
        actions,
        revision,
    })
}
