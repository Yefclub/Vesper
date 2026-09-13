//! Linux: X11 through XTest, and an honest refusal on Wayland.
//!
//! A Wayland session gives an application no way to learn which window has the
//! keyboard or to type into another one — on purpose, and GNOME and KDE hold
//! that line. XWayland would reach only the X11 applications among the user's
//! windows, which is a check that passes for some targets and not others, so
//! the whole session is treated as unsupported and the text is kept.

use crate::dictation::verdict::{any_key_down, wait_until, TypingClaim};
use crate::domain::dictation::{FailureReason, InsertionOutcome, Target};
use enigo::{Enigo, Keyboard, Settings};
use std::time::Duration;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt};

/// How long the keys of the shortcut may stay down before typing is given up.
const MODIFIER_WAIT: Duration = Duration::from_secs(2);
const POLL: Duration = Duration::from_millis(20);

fn on_wayland() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some()
        || std::env::var("XDG_SESSION_TYPE").is_ok_and(|v| v.eq_ignore_ascii_case("wayland"))
}

pub fn capture_target() -> Option<Target> {
    if on_wayland() {
        return None;
    }
    let (conn, screen) = x11rb::connect(None).ok()?;
    let root = conn.setup().roots.get(screen)?.root;
    let active = conn
        .intern_atom(false, b"_NET_ACTIVE_WINDOW")
        .ok()?
        .reply()
        .ok()?
        .atom;
    let pid_atom = conn
        .intern_atom(false, b"_NET_WM_PID")
        .ok()?
        .reply()
        .ok()?
        .atom;
    let window = conn
        .get_property(false, root, active, AtomEnum::WINDOW, 0, 1)
        .ok()?
        .reply()
        .ok()?
        .value32()?
        .next()?;
    if window == 0 {
        return None;
    }
    let pid = conn
        .get_property(false, window, pid_atom, AtomEnum::CARDINAL, 0, 1)
        .ok()?
        .reply()
        .ok()?
        .value32()
        .and_then(|mut v| v.next())
        .unwrap_or(0);
    // The window the server sends keys to: the top-level itself for most
    // toolkits, a child for the few that give a field a window of its own.
    // `None` and `PointerRoot` are 0 and 1 and name no window. Nothing on X11
    // names a field inside a window without the accessibility bus.
    let focus = conn
        .get_input_focus()
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .map_or(0, |reply| reply.focus);
    Some(Target {
        window: u64::from(window),
        control: if focus > 1 { u64::from(focus) } else { 0 },
        element: 0,
        process: pid,
        process_started: 0,
    })
}

/// Waits for every modifier key to be up — Shift, Control, Alt and the rest —
/// except Caps Lock and Num Lock, which stay "down" for as long as they are on.
///
/// A server that cannot be asked is taken as the keys being up: this is a
/// courtesy to the shortcut, not a check the insertion depends on.
pub fn wait_for_modifiers() -> bool {
    if on_wayland() {
        return true;
    }
    let Ok((conn, _)) = x11rb::connect(None) else {
        return true;
    };
    let Some(mapping) = conn
        .get_modifier_mapping()
        .ok()
        .and_then(|cookie| cookie.reply().ok())
    else {
        return true;
    };
    let per_modifier = (mapping.keycodes.len() / 8).max(1);
    // Shift, Lock, Control, Mod1..Mod5, in that order: Lock is 1 and Num Lock
    // is conventionally Mod2, which is 4.
    let held: Vec<u8> = mapping
        .keycodes
        .chunks(per_modifier)
        .enumerate()
        .filter(|(i, _)| *i != 1 && *i != 4)
        .flat_map(|(_, keycodes)| keycodes.iter().copied())
        .collect();
    wait_until(MODIFIER_WAIT, POLL, || {
        match conn
            .query_keymap()
            .ok()
            .and_then(|cookie| cookie.reply().ok())
        {
            Some(keymap) => !any_key_down(&keymap.keys, held.iter().copied()),
            None => true,
        }
    })
}

pub fn insert(_target: Target, text: &str, claim: &TypingClaim) -> InsertionOutcome {
    if on_wayland() {
        return InsertionOutcome::Failed(FailureReason::Unsupported);
    }
    let Ok(mut enigo) = Enigo::new(&Settings::default()) else {
        return InsertionOutcome::Failed(FailureReason::Failed);
    };
    if !claim.begin_typing() {
        return InsertionOutcome::Failed(FailureReason::Timeout);
    }
    // XTest delivers events to the server, not to a field, and nothing on X11
    // reads a control back without the accessibility bus — so sent is all this
    // can honestly say.
    match enigo.text(text) {
        Ok(()) => InsertionOutcome::Unconfirmed,
        Err(_) => InsertionOutcome::Failed(FailureReason::Failed),
    }
}

pub fn can_insert() -> bool {
    !on_wayland()
}

pub fn shortcut_refusal() -> Option<&'static str> {
    on_wayland().then_some("dictation.shortcut.wayland")
}

/// Wayland gives a window no say over where it is or whether it stays on top,
/// so an indicator there would land wherever the compositor put it.
pub fn floats_indicator() -> bool {
    !on_wayland()
}
