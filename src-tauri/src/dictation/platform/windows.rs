//! Windows: the foreground window and its focused control, typed into with
//! `SendInput` and confirmed by reading the control back through UI Automation.

use crate::dictation::encode::{batches, occurrences, unicode_keys};
use crate::dictation::verdict::{refuses_text, wait_until, TypingClaim};
use crate::domain::dictation::{FailureReason, InsertionOutcome, Target};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::mem::size_of;
use std::time::Duration;
use windows::core::{IUnknown, Interface};
use windows::Win32::Foundation::{CloseHandle, FILETIME, HANDLE, HWND};
use windows::Win32::Security::{
    GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, TokenIntegrityLevel,
    TOKEN_MANDATORY_LABEL, TOKEN_QUERY,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
};
use windows::Win32::System::Ole::{
    SafeArrayDestroy, SafeArrayGetElement, SafeArrayGetLBound, SafeArrayGetUBound,
};
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetProcessTimes, OpenProcess, OpenProcessToken,
    PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomation2, IUIAutomationElement, IUIAutomationTextPattern,
    IUIAutomationValuePattern, UIA_EditControlTypeId, UIA_TextPatternId, UIA_ValuePatternId,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP,
    KEYEVENTF_UNICODE, VIRTUAL_KEY, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetGUIThreadInfo, GetWindowThreadProcessId, GUITHREADINFO,
};

/// UTF-16 units per `SendInput` call. A few hundred events is what the slowest
/// ordinary target — a busy browser tab — takes without dropping any.
const BATCH_UNITS: usize = 200;

/// How long the keys of the shortcut may stay down before typing is given up.
const MODIFIER_WAIT: Duration = Duration::from_secs(2);
const POLL: Duration = Duration::from_millis(20);

/// How long UI Automation may wait on the target. A hung application answers
/// nothing, and by default it would be waited on for twenty seconds.
const UIA_TIMEOUT_MS: u32 = 1_500;

pub fn capture_target() -> Option<Target> {
    // SAFETY: queries against the window manager with out parameters owned on
    // this stack frame.
    unsafe {
        let window = GetForegroundWindow();
        if window.is_invalid() {
            return None;
        }
        let mut process = 0u32;
        let thread = GetWindowThreadProcessId(window, Some(&mut process));
        if thread == 0 || process == 0 {
            return None;
        }
        let mut info = GUITHREADINFO {
            cbSize: size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        let control = if GetGUIThreadInfo(thread, &mut info).is_ok() {
            handle_id(info.hwndFocus)
        } else {
            0
        };
        Some(Target {
            window: handle_id(window),
            control,
            element: FocusedText::new()
                .and_then(|reader| reader.focused_field())
                .unwrap_or(0),
            process,
            process_started: process_started(process).unwrap_or(0),
        })
    }
}

/// Waits for Shift, Ctrl, Alt and the Windows keys to be up.
pub fn wait_for_modifiers() -> bool {
    wait_until(MODIFIER_WAIT, POLL, || {
        [VK_SHIFT, VK_CONTROL, VK_MENU, VK_LWIN, VK_RWIN]
            .iter()
            .all(|key| {
                // SAFETY: reads the keyboard's state; nothing is passed by pointer.
                let state = unsafe { GetAsyncKeyState(i32::from(key.0)) };
                state as u16 & 0x8000 == 0
            })
    })
}

pub fn insert(target: Target, text: &str, claim: &TypingClaim) -> InsertionOutcome {
    // UIPI discards input sent to a process of higher integrity and reports
    // success anyway, so `SendInput` alone would claim text that never arrived.
    if !reachable(target.process) {
        return InsertionOutcome::Failed(FailureReason::ElevatedTarget);
    }
    let reader = FocusedText::new();
    let focused = reader.as_ref().and_then(FocusedText::inspect);
    // Bound to the field the dictation was aimed at, by the same inspection
    // that is about to be typed into. Many fields share one window handle — a
    // browser's all do — so a field that took the focus after the window was
    // checked is not the target, whatever window it sits in.
    if target.element != 0 && focused.as_ref().and_then(|f| f.element) != Some(target.element) {
        return InsertionOutcome::Failed(FailureReason::TargetChanged);
    }
    if focused
        .as_ref()
        .is_some_and(|f| refuses_text(f.control_type, f.read_only))
    {
        return InsertionOutcome::Failed(FailureReason::NoEditableTarget);
    }
    let before = focused.and_then(|f| f.text);
    if !claim.begin_typing() {
        return InsertionOutcome::Failed(FailureReason::Timeout);
    }
    for (i, batch) in batches(text, BATCH_UNITS).into_iter().enumerate() {
        let inputs: Vec<INPUT> = unicode_keys(batch)
            .into_iter()
            .map(|key| INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: VIRTUAL_KEY(0),
                        wScan: key.unit,
                        dwFlags: if key.up {
                            KEYEVENTF_UNICODE | KEYEVENTF_KEYUP
                        } else {
                            KEYEVENTF_UNICODE
                        },
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            })
            .collect();
        // SAFETY: `inputs` is a live slice of fully initialised keyboard events.
        let sent = unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) };
        if sent as usize != inputs.len() {
            // Refused before a key went in, or part of the text is already in
            // the field — and only the user can see how much.
            return if i == 0 && sent == 0 {
                InsertionOutcome::Failed(FailureReason::Failed)
            } else {
                InsertionOutcome::Unconfirmed
            };
        }
        std::thread::sleep(Duration::from_millis(15));
    }
    // The target processes its queue on its own thread; give it that long
    // before asking what the control holds.
    std::thread::sleep(Duration::from_millis(150));
    let after = reader
        .as_ref()
        .and_then(FocusedText::inspect)
        .and_then(|f| f.text);
    match (before, after) {
        (Some(before), Some(after)) if occurrences(&after, text) > occurrences(&before, text) => {
            InsertionOutcome::Inserted
        }
        _ => InsertionOutcome::Unconfirmed,
    }
}

pub fn can_insert() -> bool {
    true
}

pub fn shortcut_refusal() -> Option<&'static str> {
    None
}

pub fn floats_indicator() -> bool {
    true
}

fn handle_id(hwnd: HWND) -> u64 {
    hwnd.0 as usize as u64
}

fn process_started(pid: u32) -> Option<u64> {
    // SAFETY: the handle is opened, used and closed here, and the FILETIMEs
    // are owned on this stack frame.
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let (mut created, mut exited, mut kernel, mut user) = (
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
        );
        let ok =
            GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user).is_ok();
        let _ = CloseHandle(process);
        ok.then_some(((created.dwHighDateTime as u64) << 32) | created.dwLowDateTime as u64)
    }
}

/// Whether input sent from here can reach `pid`: its integrity level is no
/// higher than this process's. A process whose level cannot be read is treated
/// as out of reach — typing at something that cannot be inspected is the case
/// this check exists to refuse.
fn reachable(pid: u32) -> bool {
    // SAFETY: the target handle is opened and closed here; `GetCurrentProcess`
    // is a pseudo-handle that needs no closing.
    unsafe {
        let Ok(target) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return false;
        };
        let theirs = integrity(target);
        let _ = CloseHandle(target);
        let ours = integrity(GetCurrentProcess());
        matches!((theirs, ours), (Some(t), Some(o)) if t <= o)
    }
}

/// The integrity level of a process's token, as the last sub-authority of its
/// label: 0x1000 low, 0x2000 medium, 0x3000 high, 0x4000 system.
///
/// # Safety
/// `process` must be a valid process handle opened with query rights.
unsafe fn integrity(process: HANDLE) -> Option<u32> {
    let mut token = HANDLE::default();
    OpenProcessToken(process, TOKEN_QUERY, &mut token).ok()?;
    let mut needed = 0u32;
    let _ = GetTokenInformation(token, TokenIntegrityLevel, None, 0, &mut needed);
    // `u64`s rather than bytes so the buffer is aligned for the label it holds.
    let mut buffer = vec![0u64; (needed as usize).div_ceil(8).max(1)];
    let read = GetTokenInformation(
        token,
        TokenIntegrityLevel,
        Some(buffer.as_mut_ptr().cast()),
        needed,
        &mut needed,
    )
    .is_ok();
    let _ = CloseHandle(token);
    if !read {
        return None;
    }
    let label = &*(buffer.as_ptr() as *const TOKEN_MANDATORY_LABEL);
    let sid = label.Label.Sid;
    let count = *GetSidSubAuthorityCount(sid);
    if count == 0 {
        return None;
    }
    Some(*GetSidSubAuthority(sid, u32::from(count) - 1))
}

/// A UI Automation runtime id folded into one number: unique to its element for
/// as long as the element exists, which is as long as a dictation needs it.
///
/// # Safety
/// `element` must be a live UI Automation element.
unsafe fn runtime_id(element: &IUIAutomationElement) -> Option<u64> {
    let array = element.GetRuntimeId().ok()?;
    if array.is_null() {
        return None;
    }
    let mut hasher = DefaultHasher::new();
    let mut whole = true;
    match (SafeArrayGetLBound(array, 1), SafeArrayGetUBound(array, 1)) {
        (Ok(lower), Ok(upper)) => {
            for index in lower..=upper {
                let mut part = 0i32;
                if SafeArrayGetElement(array, &index, (&mut part as *mut i32).cast()).is_err() {
                    whole = false;
                    break;
                }
                part.hash(&mut hasher);
            }
        }
        _ => whole = false,
    }
    let _ = SafeArrayDestroy(array);
    if whole {
        Some(hasher.finish().max(1))
    } else {
        None
    }
}

/// What UI Automation says about the focused element.
///
/// `text` is read only to count the dictated text in it before and after, and
/// is dropped straight after the comparison — never kept, logged or sent.
struct Focused {
    control_type: Option<i32>,
    read_only: Option<bool>,
    element: Option<u64>,
    text: Option<String>,
}

struct FocusedText {
    automation: IUIAutomation,
}

impl FocusedText {
    fn new() -> Option<Self> {
        // SAFETY: COM is initialised for this thread before the object is made;
        // a thread that already had it initialised answers S_FALSE, which is fine.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let automation: IUIAutomation =
                CoCreateInstance(&CUIAutomation, None::<&IUnknown>, CLSCTX_INPROC_SERVER).ok()?;
            if let Ok(bounded) = automation.cast::<IUIAutomation2>() {
                let _ = bounded.SetConnectionTimeout(UIA_TIMEOUT_MS);
                let _ = bounded.SetTransactionTimeout(UIA_TIMEOUT_MS);
            }
            Some(Self { automation })
        }
    }

    /// The focused element's identity, when it is a field UI Automation names
    /// as one — an edit control, or anything with a value that can be written.
    ///
    /// Nothing else is named. A browser that has only just been asked answers
    /// with its whole page while it builds the tree behind it, and the field
    /// named a few seconds later would read as a different target.
    fn focused_field(&self) -> Option<u64> {
        // SAFETY: plain COM calls on interfaces owned by this struct.
        unsafe {
            let element = self.automation.GetFocusedElement().ok()?;
            let edit = element
                .CurrentControlType()
                .is_ok_and(|c| c == UIA_EditControlTypeId);
            let writable = element
                .GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
                .ok()
                .and_then(|v| v.CurrentIsReadOnly().ok())
                .is_some_and(|read_only| !read_only.as_bool());
            if edit || writable {
                runtime_id(&element)
            } else {
                None
            }
        }
    }

    fn inspect(&self) -> Option<Focused> {
        // SAFETY: plain COM calls on interfaces owned by this struct.
        unsafe {
            let element = self.automation.GetFocusedElement().ok()?;
            let control_type = element.CurrentControlType().ok().map(|c| c.0);
            let value = element
                .GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
                .ok();
            let read_only = value
                .as_ref()
                .and_then(|v| v.CurrentIsReadOnly().ok())
                .map(|b| b.as_bool());
            let text = match value.as_ref().and_then(|v| v.CurrentValue().ok()) {
                Some(v) => Some(v.to_string()),
                None => element
                    .GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId)
                    .ok()
                    .and_then(|p| p.DocumentRange().ok())
                    .and_then(|range| range.GetText(-1).ok())
                    .map(|t| t.to_string()),
            };
            Some(Focused {
                control_type,
                read_only,
                element: runtime_id(&element),
                text,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::dictation::verdict::NOT_TEXT_CONTROLS;
    use windows::Win32::UI::Accessibility::{
        UIA_ButtonControlTypeId, UIA_CheckBoxControlTypeId, UIA_HyperlinkControlTypeId,
        UIA_ImageControlTypeId, UIA_ListItemControlTypeId, UIA_MenuBarControlTypeId,
        UIA_MenuItemControlTypeId, UIA_RadioButtonControlTypeId, UIA_ScrollBarControlTypeId,
        UIA_SliderControlTypeId, UIA_TabItemControlTypeId, UIA_TreeItemControlTypeId,
    };

    /// The refused list is written as numbers so it can be tested everywhere;
    /// this holds the numbers to the SDK's names.
    #[test]
    fn the_refused_control_types_are_the_sdk_ones() {
        let sdk = [
            UIA_ButtonControlTypeId,
            UIA_CheckBoxControlTypeId,
            UIA_HyperlinkControlTypeId,
            UIA_ImageControlTypeId,
            UIA_ListItemControlTypeId,
            UIA_MenuBarControlTypeId,
            UIA_MenuItemControlTypeId,
            UIA_RadioButtonControlTypeId,
            UIA_ScrollBarControlTypeId,
            UIA_SliderControlTypeId,
            UIA_TabItemControlTypeId,
            UIA_TreeItemControlTypeId,
        ];
        assert_eq!(sdk.map(|c| c.0), NOT_TEXT_CONTROLS);
    }
}
