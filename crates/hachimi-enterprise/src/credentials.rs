use hachimi_protocol::{IntegrationProviderId, IntegrationTransport};
use serde::Deserialize;
use zeroize::Zeroize;

#[derive(Deserialize)]
#[serde(tag = "providerId", rename_all = "snake_case")]
pub enum EnterpriseCredential {
    #[serde(rename = "wecom_app")]
    WecomApp {
        #[serde(rename = "corpId")]
        corp_id: String,
        #[serde(rename = "corpSecret")]
        corp_secret: String,
        #[serde(rename = "agentId")]
        agent_id: String,
        #[serde(rename = "callbackToken")]
        callback_token: String,
        #[serde(rename = "encodingAesKey")]
        encoding_aes_key: String,
    },
    #[serde(rename = "dingtalk")]
    DingTalk {
        #[serde(rename = "clientId")]
        client_id: String,
        #[serde(rename = "clientSecret")]
        client_secret: String,
        #[serde(rename = "agentId")]
        agent_id: Option<String>,
        #[serde(rename = "robotCode")]
        robot_code: Option<String>,
    },
    Feishu {
        #[serde(rename = "appId")]
        app_id: String,
        #[serde(rename = "appSecret")]
        app_secret: String,
        #[serde(rename = "verificationToken")]
        verification_token: Option<String>,
        #[serde(rename = "encryptKey")]
        encrypt_key: Option<String>,
    },
}

impl std::fmt::Debug for EnterpriseCredential {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("EnterpriseCredential")
            .field("platform", &self.platform())
            .finish_non_exhaustive()
    }
}

impl EnterpriseCredential {
    pub fn parse(value: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(value)
    }

    #[must_use]
    pub const fn platform(&self) -> IntegrationProviderId {
        match self {
            Self::WecomApp { .. } => IntegrationProviderId::WecomApp,
            Self::DingTalk { .. } => IntegrationProviderId::DingTalk,
            Self::Feishu { .. } => IntegrationProviderId::Feishu,
        }
    }

    #[must_use]
    pub const fn ingress_mode(&self) -> IntegrationTransport {
        match self {
            Self::WecomApp { .. } => IntegrationTransport::EncryptedCallback,
            Self::DingTalk { .. } => IntegrationTransport::Stream,
            Self::Feishu { .. } => IntegrationTransport::LongConnection,
        }
    }

    #[must_use]
    pub fn tenant_id(&self) -> &str {
        match self {
            Self::WecomApp { corp_id, .. } => corp_id,
            Self::DingTalk { client_id, .. } => client_id,
            Self::Feishu { app_id, .. } => app_id,
        }
    }

    pub(crate) fn auth_pair(&self) -> (&str, &str) {
        match self {
            Self::WecomApp {
                corp_id,
                corp_secret,
                ..
            } => (corp_id, corp_secret),
            Self::DingTalk {
                client_id,
                client_secret,
                ..
            } => (client_id, client_secret),
            Self::Feishu {
                app_id, app_secret, ..
            } => (app_id, app_secret),
        }
    }

    pub(crate) fn agent_id(&self) -> Option<i64> {
        match self {
            Self::WecomApp { agent_id, .. } => agent_id.parse().ok(),
            Self::DingTalk { agent_id, .. } => agent_id.as_deref()?.parse().ok(),
            Self::Feishu { .. } => None,
        }
    }

    pub(crate) fn robot_code(&self) -> Option<&str> {
        match self {
            Self::DingTalk { robot_code, .. } => robot_code.as_deref(),
            _ => None,
        }
    }

    pub(crate) fn wecom_callback(&self) -> Option<(&str, &str)> {
        match self {
            Self::WecomApp {
                callback_token,
                encoding_aes_key,
                ..
            } => Some((callback_token, encoding_aes_key)),
            _ => None,
        }
    }

    pub(crate) fn feishu_verification_token(&self) -> Option<&str> {
        match self {
            Self::Feishu {
                verification_token, ..
            } => verification_token.as_deref(),
            _ => None,
        }
    }
}

impl Drop for EnterpriseCredential {
    fn drop(&mut self) {
        match self {
            Self::WecomApp {
                corp_id,
                corp_secret,
                callback_token,
                encoding_aes_key,
                ..
            } => {
                corp_id.zeroize();
                corp_secret.zeroize();
                callback_token.zeroize();
                encoding_aes_key.zeroize();
            }
            Self::DingTalk {
                client_id,
                client_secret,
                robot_code,
                ..
            } => {
                client_id.zeroize();
                client_secret.zeroize();
                robot_code.zeroize();
            }
            Self::Feishu {
                app_id,
                app_secret,
                verification_token,
                encrypt_key,
            } => {
                app_id.zeroize();
                app_secret.zeroize();
                verification_token.zeroize();
                encrypt_key.zeroize();
            }
        }
    }
}
