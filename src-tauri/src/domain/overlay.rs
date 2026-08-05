//! Where the minimized-recording card sits, and whether it exists at all.
//!
//! Pure geometry and one predicate, so the parts that are easy to get wrong —
//! a second monitor left of the primary, a display at 150% — are testable
//! without a window manager.

/// The card's two footprints, in logical pixels.
///
/// It steps between them rather than animating its own size: on Windows the
/// native frame and the WebView2 surface resize on different ticks, so a
/// per-frame `set_size` tears. Contents animate inside the new box, where the
/// compositor can do it at frame rate.
///
/// `COLLAPSED` is mirrored by `h-[132px] w-[14px]` in `src/overlay/Overlay.tsx`,
/// which draws the sliver at that size while the window is still at `EXPANDED`
/// waiting for the card to finish leaving. Changing it here without changing it
/// there makes the card step as it collapses.
pub const COLLAPSED: (u32, u32) = (14, 132);
pub const EXPANDED: (u32, u32) = (340, 268);

/// Whether the card should be on screen.
///
/// Both inputs are re-read from the world every time rather than remembered:
/// the recording can stop while minimized and the window can be restored while
/// recording, and a flag toggled by whichever event fired last gets one of those
/// two wrong.
pub fn overlay_visible(recording: bool, minimized: bool, focused: bool) -> bool {
    // Not focused is the real condition; minimized is one way to stop being
    // focused and was standing in for all of them. A window sitting behind the
    // call it is transcribing is exactly when a floating card earns its place,
    // and that window is neither minimized nor focused.
    recording && (minimized || !focused)
}

/// Where the card sits on the screen.
///
/// Data, not a constant, because there is no right answer: the taskbar lives
/// somewhere different on every desktop, and a card that covers the thing the
/// user is watching is worse than no card.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OverlayPosition {
    RightTop,
    #[default]
    RightCenter,
    RightBottom,
    /// Centred against the top edge, the shape of a notch. Horizontal rather
    /// than vertical, so it takes the strip of screen almost nothing else uses.
    Top,
}

impl OverlayPosition {
    pub fn from_id(id: &str) -> Self {
        match id {
            "right_top" => Self::RightTop,
            "right_bottom" => Self::RightBottom,
            "top" => Self::Top,
            _ => Self::RightCenter,
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::RightTop => "right_top",
            Self::RightCenter => "right_center",
            Self::RightBottom => "right_bottom",
            Self::Top => "top",
        }
    }

    /// Whether the card lies along the top edge rather than the right one.
    ///
    /// The two are different shapes, not the same shape in two places: a notch
    /// is wide and short, an edge dock is narrow and tall.
    pub fn is_top(self) -> bool {
        matches!(self, Self::Top)
    }
}

/// The collapsed and expanded footprints for a position.
///
/// A notch collapses to a horizontal sliver and expands downward; the edge dock
/// collapses to a vertical one and expands leftward.
pub fn footprints(position: OverlayPosition) -> ((u32, u32), (u32, u32)) {
    if position.is_top() {
        ((132, 14), (340, 268))
    } else {
        (COLLAPSED, EXPANDED)
    }
}

/// Top-left corner for a card of `size`, flush to the right edge of the monitor
/// and vertically centred, in physical pixels.
///
/// Takes the monitor's own origin, so a display placed left of the primary — a
/// negative x in the virtual desktop — is handled by arithmetic rather than by
/// assuming (0, 0). Scale is that monitor's, not the primary's, which is what a
/// mixed-DPI desktop gets wrong.
pub fn dock_at(
    position: OverlayPosition,
    monitor_pos: (i32, i32),
    monitor_size: (u32, u32),
    scale: f64,
    logical_size: (u32, u32),
) -> (i32, i32) {
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    let w = (logical_size.0 as f64 * scale).round() as i32;
    let h = (logical_size.1 as f64 * scale).round() as i32;
    let (mw, mh) = (monitor_size.0 as i32, monitor_size.1 as i32);
    // Clamped at zero everywhere: a card larger than the screen would otherwise
    // be centred off an edge, where its controls cannot be reached at all.
    // Horizontal too, not just vertical. A card wider than the monitor — a 340px
    // panel on a small display at 200% — would otherwise be placed to the LEFT
    // of that monitor's origin, which on a single-screen desktop is off the
    // screen entirely, controls and all.
    let right = (mw - w).max(0);
    let (dx, dy) = match position {
        OverlayPosition::RightTop => (right, 0),
        OverlayPosition::RightCenter => (right, ((mh - h) / 2).max(0)),
        OverlayPosition::RightBottom => (right, (mh - h).max(0)),
        OverlayPosition::Top => (((mw - w) / 2).max(0), 0),
    };
    (monitor_pos.0 + dx, monitor_pos.1 + dy)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Recording is necessary; being out of focus — by any route — is what
    /// makes the card useful. Minimizing is only one of those routes, and
    /// treating it as the only one hid the card during the case it exists for:
    /// the window sitting behind the call it is transcribing.
    #[test]
    fn the_card_shows_whenever_the_window_is_not_the_one_in_front() {
        assert!(overlay_visible(true, true, false), "minimized");
        assert!(
            overlay_visible(true, false, false),
            "open but behind something"
        );
        assert!(
            overlay_visible(true, true, true),
            "minimized wins over a stale focus flag"
        );
        assert!(
            !overlay_visible(true, false, true),
            "in front: the app itself shows this"
        );
        assert!(!overlay_visible(false, true, false), "not recording");
        assert!(!overlay_visible(false, false, false));
    }

    #[test]
    fn every_position_survives_a_round_trip_through_its_id() {
        for p in [
            OverlayPosition::RightTop,
            OverlayPosition::RightCenter,
            OverlayPosition::RightBottom,
            OverlayPosition::Top,
        ] {
            assert_eq!(OverlayPosition::from_id(p.id()), p);
        }
        // Anything else is the stored setting of a build that had other names,
        // or a value the WebView made up. It reads as the default rather than
        // leaving the card nowhere.
        assert_eq!(
            OverlayPosition::from_id("wherever"),
            OverlayPosition::RightCenter
        );
    }

    #[test]
    fn each_position_lands_on_its_own_edge() {
        let m = ((0, 0), (1920, 1080));
        let (_, expanded) = footprints(OverlayPosition::RightTop);
        let at = |p| dock_at(p, m.0, m.1, 1.0, expanded);
        assert_eq!(at(OverlayPosition::RightTop), (1920 - 340, 0));
        assert_eq!(at(OverlayPosition::RightBottom), (1920 - 340, 1080 - 268));
        let (top_x, top_y) = at(OverlayPosition::Top);
        assert_eq!(top_y, 0, "a notch hangs off the top edge");
        assert_eq!(top_x, (1920 - 340) / 2, "and is centred horizontally");
    }

    /// A card wider than the screen must still be ON the screen. Placing it at
    /// `mw - w` with a negative result puts it left of the monitor's origin,
    /// which on a single display is off it entirely.
    #[test]
    fn a_card_wider_than_the_monitor_stays_on_it() {
        for p in [
            OverlayPosition::RightTop,
            OverlayPosition::RightCenter,
            OverlayPosition::RightBottom,
            OverlayPosition::Top,
        ] {
            let (x, y) = dock_at(p, (0, 0), (200, 200), 1.0, EXPANDED);
            assert_eq!((x, y).0, 0, "{p:?} went off the left edge");
            assert!(y >= 0, "{p:?} went off the top edge");
        }
    }

    /// A notch is a different shape, not the same shape somewhere else: wide
    /// and short where the edge dock is narrow and tall.
    #[test]
    fn the_notch_collapses_horizontally() {
        let (collapsed, _) = footprints(OverlayPosition::Top);
        assert!(collapsed.0 > collapsed.1, "a notch sliver lies flat");
        let (collapsed, _) = footprints(OverlayPosition::RightCenter);
        assert!(collapsed.1 > collapsed.0, "an edge sliver stands up");
    }

    #[test]
    fn it_docks_to_the_right_edge_of_the_primary() {
        let (x, y) = dock_at(
            OverlayPosition::RightCenter,
            (0, 0),
            (1920, 1080),
            1.0,
            EXPANDED,
        );
        assert_eq!(x, 1920 - 340);
        assert_eq!(y, (1080 - 268) / 2);
    }

    /// A second display at 150%: the card has to be scaled by *that* monitor's
    /// factor, and placed inside that monitor's own coordinates.
    #[test]
    fn a_scaled_second_monitor_gets_its_own_arithmetic() {
        let (x, y) = dock_at(
            OverlayPosition::RightCenter,
            (1920, 0),
            (2560, 1440),
            1.5,
            EXPANDED,
        );
        assert_eq!(x, 1920 + 2560 - 510);
        assert_eq!(y, (1440 - 402) / 2);
    }

    /// A monitor left of the primary has a negative origin. Assuming (0, 0)
    /// would put the card on the wrong screen.
    #[test]
    fn a_monitor_left_of_the_primary_is_not_assumed_to_start_at_zero() {
        let (x, _) = dock_at(
            OverlayPosition::RightCenter,
            (-1920, 0),
            (1920, 1080),
            1.0,
            COLLAPSED,
        );
        assert_eq!(x, -1920 + 1920 - 14);
    }

    #[test]
    fn a_card_taller_than_the_screen_stays_on_it() {
        let (_, y) = dock_at(
            OverlayPosition::RightCenter,
            (0, 0),
            (800, 200),
            1.0,
            EXPANDED,
        );
        assert_eq!(y, 0, "never centred off the top edge");
    }

    /// A scale of zero or NaN is a broken answer from the platform, not a
    /// reason to place the window at the origin with no size.
    #[test]
    fn a_nonsense_scale_falls_back_to_one() {
        assert_eq!(
            dock_at(
                OverlayPosition::RightCenter,
                (0, 0),
                (1920, 1080),
                0.0,
                COLLAPSED
            ),
            dock_at(
                OverlayPosition::RightCenter,
                (0, 0),
                (1920, 1080),
                1.0,
                COLLAPSED
            )
        );
        assert_eq!(
            dock_at(
                OverlayPosition::RightCenter,
                (0, 0),
                (1920, 1080),
                f64::NAN,
                COLLAPSED
            ),
            dock_at(
                OverlayPosition::RightCenter,
                (0, 0),
                (1920, 1080),
                1.0,
                COLLAPSED
            )
        );
    }
}
