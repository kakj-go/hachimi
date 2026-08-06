use sha2::{Digest, Sha256};

const SERVICE: &str = "com.hachimi.desktop.browser-extension";
const USER: &str = "approved-installation-digest";

pub(super) fn is_trusted(identity: &str) -> bool {
    keyring::Entry::new(SERVICE, USER)
        .ok()
        .and_then(|entry| entry.get_password().ok())
        .is_some_and(|stored| stored == digest(identity))
}

pub(super) fn trust(identity: &str) -> Result<(), String> {
    keyring::Entry::new(SERVICE, USER)
        .map_err(|error| error.to_string())?
        .set_password(&digest(identity))
        .map_err(|error| error.to_string())
}

fn digest(identity: &str) -> String {
    Sha256::digest(identity.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
