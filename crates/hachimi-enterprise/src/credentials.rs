use hachimi_protocol::{EnterpriseIngressMode, EnterprisePlatform};
use serde::Deserialize;
use zeroize::Zeroize;

#[derive(Deserialize)]
#[serde(tag = "platform", rename_all = "snake_case")]
pub enum EnterpriseCredential {
    Wecom {
        #[serde(rename = "corpId")]
        corp_id: String,
        #[serde(rename = "corpSecret")]
        corp_secret: String,
        #[serde(rename = "agentId")]
        agent_id: Option<i64>,
        #[serde(rename = "callbackToken")]
        callback_token: String,
        #[serde(rename = "encodingAesKey")]
        encoding_aes_key: String,
    },
    DingTalk {
        #[serde(rename = "appKey")]
        app_key: String,
        #[serde(rename = "appSecret")]
        app_secret: String,
        #[serde(rename = "agentId")]
        agent_id: Option<i64>,
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
    pub const fn platform(&self) -> EnterprisePlatform {
        match self {
            Self::Wecom { .. } => EnterprisePlatform::Wecom,
            Self::DingTalk { .. } => EnterprisePlatform::DingTalk,
            Self::Feishu { .. } => EnterprisePlatform::Feishu,
        }
    }

    #[must_use]
    pub const fn ingress_mode(&self) -> EnterpriseIngressMode {
        match self {
            Self::Wecom { .. } => EnterpriseIngressMode::EncryptedCallback,
            Self::DingTalk { .. } => EnterpriseIngressMode::Stream,
            Self::Feishu { .. } => EnterpriseIngressMode::LongConnection,
        }
    }

    #[must_use]
    pub fn tenant_id(&self) -> &str {
        match self {
            Self::Wecom { corp_id, .. } => corp_id,
            Self::DingTalk { app_key, .. } => app_key,
            Self::Feishu { app_id, .. } => app_id,
        }
    }

    pub(crate) fn auth_pair(&self) -> (&str, &str) {
        match self {
            Self::Wecom {
                corp_id,
                corp_secret,
                ..
            } => (corp_id, corp_secret),
            Self::DingTalk {
                app_key,
                app_secret,
                ..
            } => (app_key, app_secret),
            Self::Feishu {
                app_id, app_secret, ..
            } => (app_id, app_secret),
        }
    }

    pub(crate) const fn agent_id(&self) -> Option<i64> {
        match self {
            Self::Wecom { agent_id, .. } | Self::DingTalk { agent_id, .. } => *agent_id,
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
            Self::Wecom {
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
            Self::Wecom {
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
                app_key,
                app_secret,
                robot_code,
                ..
            } => {
                app_key.zeroize();
                app_secret.zeroize();
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
