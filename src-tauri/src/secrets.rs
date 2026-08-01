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

/// Stores the key. Failure propagates: the alternative is the user believing their
/// key was saved and finding it gone next launch.
pub fn store_openrouter_key(key: &str) -> Result<(), String> {
    entry()
        .and_then(|e| e.set_password(key))
        .map_err(|e| format!("could not store the API key in the system keychain: {e}"))
}

/// Removes the stored key, and fails loudly if it cannot.
///
/// Call this only when a key is known to be set — the caller knows that from its
/// own state, which is the one source that does not lie when the keychain is
/// locked. Asking the keychain whether something is stored cannot distinguish
/// "nothing here" from "cannot answer right now", and guessing either way is
/// wrong: guess absent and a revoked credential quietly comes back to life when
/// the service recovers; guess present and someone on a machine with no keychain
/// at all cannot finish onboarding over a key they never had.
pub fn clear_openrouter_key() -> Result<(), String> {
    match entry().and_then(|e| e.delete_credential()) {
        // Already gone, including the race where another process removed it first.
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(format!("could not remove the stored API key: {e}")),
    }
}
