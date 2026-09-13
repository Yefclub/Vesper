//! What differs between desktops: naming where the keyboard is, and typing
//! into it.
//!
//! Every platform answers the same questions. A desktop that gives an
//! application no way to type into another answers them honestly rather than
//! pretending.

#[cfg(target_os = "linux")]
mod linux;
// Compiled by every platform's tests as well. It is all `osascript` and the
// standard library, so it builds anywhere — and no CI job builds for macOS, so
// this is the only place its scripts are ever checked before a Mac runs them.
#[cfg(any(target_os = "macos", test))]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
mod macos;
#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
mod unsupported;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "linux")]
pub use linux::{
    can_insert, capture_target, floats_indicator, insert, shortcut_refusal, wait_for_modifiers,
};
#[cfg(target_os = "macos")]
pub use macos::{
    can_insert, capture_target, floats_indicator, insert, shortcut_refusal, wait_for_modifiers,
};
#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
pub use unsupported::{
    can_insert, capture_target, floats_indicator, insert, shortcut_refusal, wait_for_modifiers,
};
#[cfg(windows)]
pub use windows::{
    can_insert, capture_target, floats_indicator, insert, shortcut_refusal, wait_for_modifiers,
};
