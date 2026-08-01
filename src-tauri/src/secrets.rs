//! OS keychain storage for the user's API key.
//!
//! The key used to live in `vesper.db` as plain JSON, readable by any process
//! running as the user. For an app whose selling point is that nothing leaves the
//! machine, the one credential it does hold is the worst thing to leave lying in
//! cleartext. It now goes to the Windows Credential Manager, the macOS Keychain
//! or the Secret Service on Linux.
//!
//! Every operation is fallible and none of them are fatal: a machine with no
//! keychain daemon — a bare Linux container, say — must still run the app, just
//! without remembering the key between launches. Failures are logged without the
//! value.

const SERVICE: &str = "com.yefclub.vesper";
const OPENROUTER_ACCOUNT: &str = "openrouter-api-key";

fn entry() -> Result<keyring::Entry, keyring::Error> {
    keyring::Entry::new(SERVICE, OPENROUTER_ACCOUNT)
}

/// Reads the stored key. `None` covers both "never stored" and "keychain
/// unavailable" — the caller treats them the same way, by asking the user again.
pub fn openrouter_key() -> Option<String> {
    match entry().and_then(|e| e.get_password()) {
        Ok(k) if !k.trim().is_empty() => Some(k),
        Ok(_) => None,
        Err(keyring::Error::NoEntry) => None,
        Err(e) => {
            tracing::warn!("keychain read failed: {e}");
            None
        }
    }
}

/// Stores the key, or clears it when `None` or blank.
///
/// Clearing never fails the caller. There is nothing to lose by failing to delete
/// something, and every settings save passes through here — including a local-only
/// setup that has no key at all. Returning an error there would stop someone on a
/// machine without a keychain daemon from finishing onboarding, over a credential
/// they never had.
///
/// Storing a real key does propagate the error, because the alternative is the
/// user believing their key was saved and finding it gone next launch.
pub fn set_openrouter_key(key: Option<&str>) -> Result<(), String> {
    match key.map(str::trim).filter(|k| !k.is_empty()) {
        Some(k) => entry()
            .and_then(|e| e.set_password(k))
            .map_err(|e| format!("could not store the API key in the system keychain: {e}")),
        None => {
            if let Err(e) = entry().and_then(|e| e.delete_credential()) {
                if !matches!(e, keyring::Error::NoEntry) {
                    tracing::warn!("could not clear the stored API key: {e}");
                }
            }
            Ok(())
        }
    }
}
