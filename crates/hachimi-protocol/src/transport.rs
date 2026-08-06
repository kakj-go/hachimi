use hachimi_core::WindowKind;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
pub struct RequestId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
pub struct ClientId(pub String);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ControlRequest<T> {
    pub protocol_version: u32,
    pub id: RequestId,
    pub client_id: ClientId,
    pub method: String,
    pub params: T,
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ControlResponse<T> {
    pub id: RequestId,
    pub ok: bool,
    pub payload: Option<T>,
    pub error: Option<ControlError>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ControlEvent<T> {
    pub event: String,
    pub payload: T,
    pub seq: u64,
    pub state_version: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ControlErrorCode {
    InvalidProtocolVersion,
    InvalidRequest,
    UnknownMethod,
    PermissionDenied,
    ApprovalRequired,
    FeatureDisabled,
    Conflict,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ControlError {
    pub code: ControlErrorCode,
    pub message: String,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Type,
)]
pub enum Scope {
    #[serde(rename = "pet.interact")]
    PetInteract,
    #[serde(rename = "agent.run")]
    AgentRun,
    #[serde(rename = "settings.read")]
    SettingsRead,
    #[serde(rename = "settings.write")]
    SettingsWrite,
    #[serde(rename = "llm.read")]
    LlmRead,
    #[serde(rename = "llm.write")]
    LlmWrite,
    #[serde(rename = "llm.test")]
    LlmTest,
    #[serde(rename = "llm.chat")]
    LlmChat,
    #[serde(rename = "avatar.read")]
    AvatarRead,
    #[serde(rename = "avatar.manage")]
    AvatarManage,
    #[serde(rename = "avatar.runtime")]
    AvatarRuntime,
    #[serde(rename = "motion.read")]
    MotionRead,
    #[serde(rename = "motion.manage")]
    MotionManage,
    #[serde(rename = "motion.runtime")]
    MotionRuntime,
    #[serde(rename = "voice.read")]
    VoiceRead,
    #[serde(rename = "voice.manage")]
    VoiceManage,
    #[serde(rename = "voice.playback")]
    VoicePlayback,
    #[serde(rename = "voice.capture")]
    VoiceCapture,
    #[serde(rename = "workbench.open")]
    WorkbenchOpen,
    #[serde(rename = "workbench.window")]
    WorkbenchWindow,
    #[serde(rename = "workspace.read")]
    WorkspaceRead,
    #[serde(rename = "workspace.write")]
    WorkspaceWrite,
    #[serde(rename = "workspace.exec")]
    WorkspaceExec,
    #[serde(rename = "browser.observe")]
    BrowserObserve,
    #[serde(rename = "browser.control")]
    BrowserControl,
    #[serde(rename = "computer.observe")]
    ComputerObserve,
    #[serde(rename = "computer.control")]
    ComputerControl,
    #[serde(rename = "connectors.invoke")]
    ConnectorsInvoke,
    #[serde(rename = "connectors.manage")]
    ConnectorsManage,
    #[serde(rename = "channels.manage")]
    ChannelsManage,
    #[serde(rename = "gateway.manage")]
    GatewayManage,
    #[serde(rename = "skills.manage")]
    SkillsManage,
    #[serde(rename = "skills.use")]
    SkillsUse,
    #[serde(rename = "devices.pair")]
    DevicesPair,
    #[serde(rename = "admin.policy")]
    AdminPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientContext {
    pub client_id: ClientId,
    pub window_kind: WindowKind,
    pub scopes: BTreeSet<Scope>,
}

impl ClientContext {
    #[must_use]
    pub fn for_window(window_kind: WindowKind) -> Self {
        let scopes = match window_kind {
            WindowKind::Pet => BTreeSet::from([
                Scope::PetInteract,
                Scope::WorkbenchOpen,
                Scope::LlmChat,
                Scope::AvatarRuntime,
                Scope::MotionRuntime,
                Scope::VoicePlayback,
                Scope::VoiceCapture,
            ]),
            WindowKind::Settings => BTreeSet::from([Scope::SettingsRead, Scope::SettingsWrite]),
            WindowKind::Workbench => BTreeSet::from([
                Scope::SettingsRead,
                Scope::SettingsWrite,
                Scope::LlmRead,
                Scope::LlmWrite,
                Scope::LlmTest,
                Scope::AvatarRead,
                Scope::AvatarManage,
                Scope::MotionRead,
                Scope::MotionManage,
                Scope::VoiceRead,
                Scope::VoiceManage,
                Scope::VoicePlayback,
                Scope::WorkbenchWindow,
                Scope::ConnectorsManage,
                Scope::BrowserObserve,
                Scope::BrowserControl,
                Scope::ComputerObserve,
                Scope::ComputerControl,
                Scope::ChannelsManage,
                Scope::GatewayManage,
                Scope::SkillsManage,
                Scope::SkillsUse,
            ]),
            WindowKind::Service => BTreeSet::new(),
        };
        Self {
            client_id: ClientId(format!("window:{}", window_kind.label())),
            window_kind,
            scopes,
        }
    }

    #[must_use]
    pub fn for_internal(principal: &str) -> Self {
        Self {
            client_id: ClientId(format!("service:{principal}")),
            window_kind: WindowKind::Service,
            scopes: BTreeSet::new(),
        }
    }
}
