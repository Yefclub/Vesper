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
        None => clear_openrouter_key(),
    }
}

/// Removing a key is only best-effort when there was nothing to remove.
///
/// If a key really is stored and the keychain refuses to delete it, saying "saved"
/// would be a lie with teeth: the user believes they revoked their credential, the
/// settings screen shows it gone, and the next launch loads it straight back out of
/// the keychain and keeps sending it to OpenRouter.
fn clear_openrouter_key() -> Result<(), String> {
    let Ok(entry) = entry() else {
        // No keychain backend at all — nothing of ours can be sitting in it.
        return Ok(());
    };
    match entry.get_password() {
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => {
            // Service unavailable or locked. We never managed to store anything
            // through it either, so there is nothing to strand.
            tracing::warn!("keychain unreadable while clearing the API key: {e}");
            Ok(())
        }
        Ok(_) => entry
            .delete_credential()
            .map_err(|e| format!("could not remove the stored API key: {e}")),
    }
}
