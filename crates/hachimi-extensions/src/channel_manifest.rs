use std::{fs, path::PathBuf};

use hachimi_protocol::{
    ChannelProviderManifest, ChannelProviderRuntimeKind, InstalledPlugin, PluginContributionKind,
};
use serde::Deserialize;
use serde_json::json;

use crate::{ExtensionHostError, safe_relative_path, value_hash};

#[derive(Debug, Clone)]
pub struct ChannelSidecarDefinition {
    pub manifest: ChannelProviderManifest,
    pub bundle_root: PathBuf,
    pub executable: PathBuf,
    pub args: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ChannelSidecarDescriptor {
    protocol_version: u32,
    provider_id: String,
    transport: String,
    entrypoint: String,
    #[serde(default)]
    args: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BuiltinEnterpriseChannelDescriptor {
    protocol_version: u32,
    provider_id: String,
    transport: String,
}

pub(crate) fn builtin_enterprise_channel_provider_id(
    plugin: &InstalledPlugin,
    contribution_id: &str,
) -> Result<Option<String>, ExtensionHostError> {
    let contribution = plugin
        .manifest
        .contributions
        .iter()
        .find(|contribution| {
            contribution.kind == PluginContributionKind::Channel
                && contribution.id == contribution_id
        })
        .ok_or(ExtensionHostError::ContributionDrift)?;
    let bundle_root = PathBuf::from(&plugin.root_path)
        .canonicalize()
        .map_err(|_| ExtensionHostError::ContributionDrift)?;
    let descriptor_path = bundle_root
        .join(safe_relative_path(&contribution.relative_path)?)
        .canonicalize()
        .map_err(|_| ExtensionHostError::ContributionDrift)?;
    if !descriptor_path.starts_with(&bundle_root) || !descriptor_path.is_file() {
        return Err(ExtensionHostError::ContributionEscape);
    }
    let bytes = fs::read(descriptor_path)?;
    let value: serde_json::Value = serde_json::from_slice(&bytes)?;
    if value.get("transport").and_then(serde_json::Value::as_str) != Some("builtin_enterprise") {
        return Ok(None);
    }
    let descriptor: BuiltinEnterpriseChannelDescriptor = serde_json::from_value(value)?;
    let provider_id = plugin.manifest.id.as_str();
    let expected_contribution_id = format!("{}-channel", provider_id.replace('_', "-"));
    if descriptor.protocol_version != 1
        || descriptor.transport != "builtin_enterprise"
        || descriptor.provider_id != provider_id
        || contribution_id != expected_contribution_id
        || !matches!(
            provider_id,
            "dingtalk" | "feishu" | "wecom_ai_bot" | "wecom_app" | "wechat_ilink"
        )
    {
        return Err(ExtensionHostError::InvalidManifest(
            "builtin enterprise channel identity is invalid".into(),
        ));
    }
    Ok(Some(descriptor.provider_id))
}

pub(crate) fn channel_sidecar_definition(
    plugin: &InstalledPlugin,
    contribution_id: &str,
) -> Result<ChannelSidecarDefinition, ExtensionHostError> {
    let contribution = plugin
        .manifest
        .contributions
        .iter()
        .find(|contribution| {
            contribution.kind == PluginContributionKind::Channel
                && contribution.id == contribution_id
        })
        .ok_or(ExtensionHostError::ContributionDrift)?;
    let bundle_root = PathBuf::from(&plugin.root_path)
        .canonicalize()
        .map_err(|_| ExtensionHostError::ContributionDrift)?;
    let descriptor_path = bundle_root
        .join(safe_relative_path(&contribution.relative_path)?)
        .canonicalize()
        .map_err(|_| ExtensionHostError::ContributionDrift)?;
    if !descriptor_path.starts_with(&bundle_root) || !descriptor_path.is_file() {
        return Err(ExtensionHostError::ContributionEscape);
    }
    let bytes = fs::read(&descriptor_path)?;
    let descriptor: ChannelSidecarDescriptor = serde_json::from_slice(&bytes)?;
    let expected_provider_id =
        format!("plugin.{}.{}", plugin.manifest.id.as_str(), contribution.id);
    if descriptor.protocol_version != 1
        || descriptor.transport != "stdio_json_rpc"
        || descriptor.provider_id != expected_provider_id
        || descriptor.provider_id.len() > 128
        || descriptor
            .args
            .iter()
            .any(|argument| argument.len() > 4_096 || argument.contains('\0'))
    {
        return Err(ExtensionHostError::InvalidManifest(
            "channel sidecar descriptor is invalid or not plugin-namespaced".into(),
        ));
    }
    let executable = bundle_root
        .join(safe_relative_path(&descriptor.entrypoint)?)
        .canonicalize()
        .map_err(|_| ExtensionHostError::ContributionDrift)?;
    if !executable.starts_with(&bundle_root) || !executable.is_file() {
        return Err(ExtensionHostError::ContributionEscape);
    }
    let content_hash = value_hash(&json!({
        "pluginContentHash": plugin.content_hash,
        "contributionId": contribution.id,
        "descriptor": serde_json::from_slice::<serde_json::Value>(&bytes)?,
    }))?;
    Ok(ChannelSidecarDefinition {
        manifest: ChannelProviderManifest {
            id: descriptor.provider_id,
            plugin_id: Some(plugin.manifest.id.clone()),
            runtime_kind: ChannelProviderRuntimeKind::SandboxedStdioJsonRpc,
            entrypoint: Some(contribution.relative_path.clone()),
            content_hash,
            required_scopes: contribution.required_scopes.clone(),
        },
        bundle_root,
        executable,
        args: descriptor.args,
    })
}
