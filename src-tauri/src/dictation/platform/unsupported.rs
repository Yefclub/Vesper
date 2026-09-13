//! A desktop this build has no way to type into.

use crate::dictation::verdict::TypingClaim;
use crate::domain::dictation::{FailureReason, InsertionOutcome, Target};

pub fn capture_target() -> Option<Target> {
    None
}

pub fn wait_for_modifiers() -> bool {
    true
}

pub fn insert(_target: Target, _text: &str, _claim: &TypingClaim) -> InsertionOutcome {
    InsertionOutcome::Failed(FailureReason::Unsupported)
}

pub fn can_insert() -> bool {
    false
}

pub fn shortcut_refusal() -> Option<&'static str> {
    None
}

pub fn floats_indicator() -> bool {
    false
}
