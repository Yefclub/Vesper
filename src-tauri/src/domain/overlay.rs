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
pub fn overlay_visible(recording: bool, minimized: bool) -> bool {
    recording && minimized
}

/// Top-left corner for a card of `size`, flush to the right edge of the monitor
/// and vertically centred, in physical pixels.
///
/// Takes the monitor's own origin, so a display placed left of the primary — a
/// negative x in the virtual desktop — is handled by arithmetic rather than by
/// assuming (0, 0). Scale is that monitor's, not the primary's, which is what a
/// mixed-DPI desktop gets wrong.
pub fn dock_right_center(
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
    let x = monitor_pos.0 + monitor_size.0 as i32 - w;
    // Clamped to the top of the monitor: a card taller than the screen would
    // otherwise be centred off the top edge, where its controls cannot be
    // reached at all.
    let y = monitor_pos.1 + ((monitor_size.1 as i32 - h) / 2).max(0);
    (x, y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_card_needs_both_conditions() {
        assert!(overlay_visible(true, true));
        assert!(!overlay_visible(true, false));
        assert!(!overlay_visible(false, true));
        assert!(!overlay_visible(false, false));
    }

    #[test]
    fn it_docks_to_the_right_edge_of_the_primary() {
        let (x, y) = dock_right_center((0, 0), (1920, 1080), 1.0, EXPANDED);
        assert_eq!(x, 1920 - 340);
        assert_eq!(y, (1080 - 268) / 2);
    }

    /// A second display at 150%: the card has to be scaled by *that* monitor's
    /// factor, and placed inside that monitor's own coordinates.
    #[test]
    fn a_scaled_second_monitor_gets_its_own_arithmetic() {
        let (x, y) = dock_right_center((1920, 0), (2560, 1440), 1.5, EXPANDED);
        assert_eq!(x, 1920 + 2560 - 510);
        assert_eq!(y, (1440 - 402) / 2);
    }

    /// A monitor left of the primary has a negative origin. Assuming (0, 0)
    /// would put the card on the wrong screen.
    #[test]
    fn a_monitor_left_of_the_primary_is_not_assumed_to_start_at_zero() {
        let (x, _) = dock_right_center((-1920, 0), (1920, 1080), 1.0, COLLAPSED);
        assert_eq!(x, -1920 + 1920 - 14);
    }

    #[test]
    fn a_card_taller_than_the_screen_stays_on_it() {
        let (_, y) = dock_right_center((0, 0), (800, 200), 1.0, EXPANDED);
        assert_eq!(y, 0, "never centred off the top edge");
    }

    /// A scale of zero or NaN is a broken answer from the platform, not a
    /// reason to place the window at the origin with no size.
    #[test]
    fn a_nonsense_scale_falls_back_to_one() {
        assert_eq!(
            dock_right_center((0, 0), (1920, 1080), 0.0, COLLAPSED),
            dock_right_center((0, 0), (1920, 1080), 1.0, COLLAPSED)
        );
        assert_eq!(
            dock_right_center((0, 0), (1920, 1080), f64::NAN, COLLAPSED),
            dock_right_center((0, 0), (1920, 1080), 1.0, COLLAPSED)
        );
    }
}
