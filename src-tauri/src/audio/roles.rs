//! Which output Windows considers "the call".
//!
//! Windows assigns three roles to an endpoint, and its own documentation is the
//! whole argument for this file: `eConsole` is *"games, system notification
//! sounds, and voice commands"*, while `eCommunications` is *"voice
//! communications (talking to another person)"*. A meeting is the second one.
//!
//! They are frequently different devices, and nothing warns anybody. On the
//! machine this was found on, `eConsole` was the built-in Realtek speakers and
//! `eCommunications` a USB headset — so the loopback recorded the speakers
//! while the call played through the headset, and every meeting came back with
//! the other people missing. The user's own microphone was fine, which is what
//! made it look like the application worked.
//!
//! The audio backend resolves an unspecified device with
//! `GetDefaultAudioEndpoint(eRender, eConsole)`. For a meeting recorder that is
//! the wrong of the two, so the id is resolved here and passed explicitly.

/// The output device Windows routes voice calls to, as a device id the audio
/// backend accepts.
///
/// `None` on failure and on every other platform, which the caller reads as
/// "let the backend pick" — the behaviour before this existed.
#[cfg(windows)]
pub fn communications_render_id() -> Option<String> {
    default_render_id(windows::Win32::Media::Audio::eCommunications)
}

#[cfg(not(windows))]
pub fn communications_render_id() -> Option<String> {
    None
}

/// The output device Windows routes games and notifications to.
///
/// Only worth knowing to compare against the one above: equal means this whole
/// module changed nothing, and different means a meeting would have been
/// recorded silent.
#[cfg(windows)]
pub fn console_render_id() -> Option<String> {
    default_render_id(windows::Win32::Media::Audio::eConsole)
}

#[cfg(not(windows))]
pub fn console_render_id() -> Option<String> {
    None
}

#[cfg(windows)]
fn default_render_id(role: windows::Win32::Media::Audio::ERole) -> Option<String> {
    use windows::core::PCWSTR;
    use windows::Win32::Media::Audio::{eRender, IMMDeviceEnumerator, MMDeviceEnumerator};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED,
    };

    unsafe {
        // Ignored on purpose: a thread already initialised for a different
        // apartment answers `RPC_E_CHANGED_MODE`, and the enumerator works
        // either way. Failing here would turn a working machine into a silent
        // one over a return code that means "somebody got here first".
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
        let device = enumerator.GetDefaultAudioEndpoint(eRender, role).ok()?;
        let id = device.GetId().ok()?;
        let owned = PCWSTR(id.0).to_string().ok();
        // `GetId` hands back memory the caller owns.
        windows::Win32::System::Com::CoTaskMemFree(Some(id.0 as *const _));
        owned
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Not an assertion about this machine — it is an assertion that the lookup
    /// works at all. A `None` here on Windows means the COM call failed, and the
    /// caller would silently fall back to the console endpoint, which is the bug
    /// this module exists to fix.
    #[test]
    #[cfg(windows)]
    fn windows_answers_for_both_roles() {
        assert!(
            communications_render_id().is_some(),
            "no communications endpoint — the loopback would fall back to eConsole"
        );
        assert!(console_render_id().is_some());
    }

    /// The reason the module exists, printed rather than asserted: whether the
    /// two differ is a property of the machine, and a test that demanded either
    /// answer would fail on half the world.
    #[test]
    #[cfg(windows)]
    fn report_whether_the_roles_disagree() {
        let call = communications_render_id();
        let console = console_render_id();
        println!("eCommunications: {call:?}");
        println!("eConsole:        {console:?}");
        println!(
            "roles disagree:  {}",
            if call == console {
                "no"
            } else {
                "YES — a meeting would record silent without this"
            }
        );
    }
}
