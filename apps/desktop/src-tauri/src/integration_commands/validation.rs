use hachimi_protocol::{
    IntegrationAccountUpsert, IntegrationCredentialInput, IntegrationProviderId,
};

use crate::CommandError;

pub(super) fn validate_upsert(input: &IntegrationAccountUpsert) -> Result<(), CommandError> {
    if input.id.trim().is_empty()
        || input.id.len() > 128
        || input.display_name.trim().is_empty()
        || input.display_name.len() > 160
    {
        return Err(CommandError::new(
            "integration_input_invalid",
            "Integration ID and display name are required.",
        ));
    }
    validate_capabilities(
        input.credential.provider_id(),
        input.api_access_enabled,
        input.messaging_enabled,
    )?;
    if !credential_shape_valid(&input.credential, input.messaging_enabled) {
        return Err(CommandError::new(
            "integration_credentials_invalid",
            "The required platform credentials are missing or invalid.",
        ));
    }
    Ok(())
}

pub(super) fn credential_shape_valid(
    input: &IntegrationCredentialInput,
    messaging_enabled: bool,
) -> bool {
    let present = |value: &str| !value.trim().is_empty() && value.len() <= 64 * 1024;
    match input {
        IntegrationCredentialInput::DingTalk {
            client_id,
            client_secret,
            ..
        } => present(client_id) && present(client_secret),
        IntegrationCredentialInput::Feishu { app_id, app_secret } => {
            present(app_id) && present(app_secret)
        }
        IntegrationCredentialInput::WecomAiBot { bot_id, secret } => {
            present(bot_id) && present(secret)
        }
        IntegrationCredentialInput::WecomApp {
            corp_id,
            corp_secret,
            agent_id,
            callback_token,
            encoding_aes_key,
            external_https_url,
        } => {
            present(corp_id)
                && present(corp_secret)
                && present(agent_id)
                && (!messaging_enabled
                    || (present(callback_token)
                        && present(encoding_aes_key)
                        && valid_https_url(external_https_url)))
        }
        IntegrationCredentialInput::WechatIlink {
            bot_token,
            bot_id,
            base_url,
        } => {
            present(bot_token)
                && present(bot_id)
                && hachimi_channel_providers::WechatIlinkAdapter::validate_base_url(base_url)
                    .is_ok()
        }
    }
}

pub(super) fn validate_capabilities(
    provider_id: IntegrationProviderId,
    api: bool,
    messaging: bool,
) -> Result<(), CommandError> {
    if !api && !messaging {
        return Err(CommandError::new(
            "integration_capability_required",
            "Select API access, messaging, or both.",
        ));
    }
    if api && !provider_id.supports_enterprise_api() {
        return Err(CommandError::new(
            "integration_api_unsupported",
            "This Provider does not expose an enterprise API Connector.",
        ));
    }
    Ok(())
}

fn valid_https_url(value: &str) -> bool {
    url::Url::parse(value).ok().is_some_and(|url| {
        url.scheme() == "https"
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
    })
}
