use hachimi_protocol::ConnectorAccountId;

use crate::ExtensionHostError;

pub(crate) fn connector_secret_reference(account_id: &ConnectorAccountId) -> String {
    format!("keyring:connector:{}", account_id.as_str())
}

pub(crate) fn connector_keyring_entry(
    account_id: &ConnectorAccountId,
) -> Result<keyring::Entry, ExtensionHostError> {
    keyring::Entry::new("com.hachimi.connector", account_id.as_str())
        .map_err(|_| ExtensionHostError::SecretStore)
}

pub(crate) fn connector_secret(
    reference: &str,
    account_id: &ConnectorAccountId,
) -> Result<Option<String>, ExtensionHostError> {
    if reference == connector_secret_reference(account_id) {
        return read_entry(connector_keyring_entry(account_id)?);
    }
    let Some(identity) = reference.strip_prefix("keyring:integration:") else {
        return Err(ExtensionHostError::SecretStore);
    };
    let Some(identity) = identity.strip_suffix(":primary") else {
        return Err(ExtensionHostError::SecretStore);
    };
    let Some((provider_id, integration_account_id)) = identity.split_once(':') else {
        return Err(ExtensionHostError::SecretStore);
    };
    if !matches!(provider_id, "dingtalk" | "feishu" | "wecom_app")
        || integration_account_id.is_empty()
        || integration_account_id.len() > 128
    {
        return Err(ExtensionHostError::SecretStore);
    }
    let entry = keyring::Entry::new(
        "com.hachimi.integration",
        &format!("{provider_id}:{integration_account_id}:primary"),
    )
    .map_err(|_| ExtensionHostError::SecretStore)?;
    read_entry(entry)
}

fn read_entry(entry: keyring::Entry) -> Result<Option<String>, ExtensionHostError> {
    match entry.get_password() {
        Ok(secret) => Ok(Some(secret)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(_) => Err(ExtensionHostError::SecretStore),
    }
}
