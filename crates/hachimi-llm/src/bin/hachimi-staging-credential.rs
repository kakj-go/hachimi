use std::io::{self, Read};

use zeroize::Zeroize;

const ALLOWED_SERVICES: &[&str] = &[
    "com.hachimi.desktop",
    "com.hachimi.forge",
    "com.hachimi.connector",
    "com.hachimi.channel",
];

fn valid_account(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 512
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.' | ':')
        })
}

trait CredentialEntry {
    fn set(&self, secret: &str) -> Result<(), ()>;
    fn clear(&self) -> Result<(), ()>;
    fn present(&self) -> Result<bool, ()>;
}

struct SystemCredentialEntry(keyring::Entry);

impl CredentialEntry for SystemCredentialEntry {
    fn set(&self, secret: &str) -> Result<(), ()> {
        self.0.set_password(secret).map_err(|_| ())
    }

    fn clear(&self) -> Result<(), ()> {
        match self.0.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(()),
        }
    }

    fn present(&self) -> Result<bool, ()> {
        match self.0.get_password() {
            Ok(mut secret) => {
                secret.zeroize();
                Ok(true)
            }
            Err(keyring::Error::NoEntry) => Ok(false),
            Err(_) => Err(()),
        }
    }
}

fn execute(
    entry: &dyn CredentialEntry,
    action: &str,
    secret: Option<&str>,
) -> Result<(), &'static str> {
    match action {
        "set" => entry
            .set(secret.ok_or("staging_credential_value_rejected")?)
            .map_err(|_| "staging_credential_store_failed"),
        "clear" => entry.clear().map_err(|_| "staging_credential_clear_failed"),
        "assert-present" => entry
            .present()
            .map_err(|_| "staging_credential_store_unavailable")?
            .then_some(())
            .ok_or("staging_credential_missing"),
        "assert-absent" => (!entry
            .present()
            .map_err(|_| "staging_credential_store_unavailable")?)
        .then_some(())
        .ok_or("staging_credential_not_cleared"),
        _ => Err("staging_credential_action_invalid"),
    }
}

fn main() {
    if let Err(code) = run() {
        eprintln!("{code}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), &'static str> {
    let mut arguments = std::env::args().skip(1);
    let action = arguments
        .next()
        .ok_or("staging_credential_action_missing")?;
    let service = arguments
        .next()
        .ok_or("staging_credential_service_missing")?;
    let account = arguments
        .next()
        .ok_or("staging_credential_account_missing")?;
    if arguments.next().is_some() {
        return Err("staging_credential_argument_unexpected");
    }
    if !ALLOWED_SERVICES.contains(&service.as_str()) || !valid_account(&account) {
        return Err("staging_credential_target_rejected");
    }
    if service == "com.hachimi.desktop" && account != "llm-api-key" {
        return Err("staging_credential_target_rejected");
    }
    let entry = SystemCredentialEntry(
        keyring::Entry::new(&service, &account)
            .map_err(|_| "staging_credential_store_unavailable")?,
    );
    let mut secret = None;
    if action == "set" {
        let mut value = String::new();
        io::stdin()
            .take(1_048_577)
            .read_to_string(&mut value)
            .map_err(|_| "staging_credential_stdin_failed")?;
        while matches!(value.chars().last(), Some('\r' | '\n')) {
            value.pop();
        }
        if value.is_empty() || value.len() > 1_048_576 {
            value.zeroize();
            return Err("staging_credential_value_rejected");
        }
        secret = Some(value);
    }
    let result = execute(&entry, &action, secret.as_deref());
    if let Some(value) = secret.as_mut() {
        value.zeroize();
    }
    result
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::{CredentialEntry, execute, valid_account};

    #[derive(Default)]
    struct MemoryEntry(Mutex<Option<String>>);

    impl CredentialEntry for MemoryEntry {
        fn set(&self, secret: &str) -> Result<(), ()> {
            *self.0.lock().expect("entry") = Some(secret.to_owned());
            Ok(())
        }

        fn clear(&self) -> Result<(), ()> {
            self.0.lock().expect("entry").take();
            Ok(())
        }

        fn present(&self) -> Result<bool, ()> {
            Ok(self.0.lock().expect("entry").is_some())
        }
    }

    #[test]
    fn staging_accounts_are_bounded_and_path_free() {
        assert!(valid_account("release-github"));
        assert!(valid_account("wecom:release"));
        assert!(!valid_account("../escape"));
        assert!(!valid_account("with space"));
    }

    #[test]
    fn temporary_credential_is_set_verified_cleared_and_verified_absent() {
        let entry = MemoryEntry::default();
        execute(&entry, "assert-absent", None).expect("initially absent");
        execute(&entry, "set", Some("temporary-secret")).expect("set");
        execute(&entry, "assert-present", None).expect("present");
        execute(&entry, "clear", None).expect("clear");
        execute(&entry, "assert-absent", None).expect("absent");
    }

    #[test]
    fn assertions_fail_closed_without_disclosing_secret_values() {
        let entry = MemoryEntry::default();
        assert_eq!(
            execute(&entry, "assert-present", None),
            Err("staging_credential_missing")
        );
        execute(&entry, "set", Some("must-not-be-returned")).expect("set");
        assert_eq!(
            execute(&entry, "assert-absent", None),
            Err("staging_credential_not_cleared")
        );
    }
}
