//! Fixed-endpoint clients and ingress verification for WeCom, DingTalk, and Feishu.

mod api;
mod credentials;
mod events;
mod stream;

pub use api::{
    EnterpriseApiClient, EnterpriseApiError, EnterpriseDirectoryPage, EnterpriseDownloadReceipt,
    EnterpriseMediaKind, EnterpriseMessageTarget,
};
pub use credentials::EnterpriseCredential;
pub use events::{
    EnterpriseEventAuth, EnterpriseEventError, EnterpriseRawEvent, VerifiedEnterpriseEvent,
    verify_enterprise_event, verify_wecom_callback_echo,
};
pub use stream::{
    EnterpriseStreamEndpoint, EnterpriseStreamEvent, EnterpriseStreamRuntime,
    spawn_enterprise_stream,
};
