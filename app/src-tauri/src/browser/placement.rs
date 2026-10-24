//! Where the AI browser's window opens: on the display of the Debrief window that started it, centred on that window
//! and kept inside the display's work area (never on "the primary display" unless that is where the window is).
//!
//! Pure geometry. Displays and the owner window come in physical pixels (what Tauri reports), so displays with
//! different scale factors and displays left of or above the primary one (negative coordinates) are compared
//! correctly. The result is in the units the browser reads `--window-position` and `--window-size` in: the chosen
//! display's physical pixels divided by its scale (points on macOS; device-independent pixels elsewhere, where the
//! browser's own layout of mixed-scale displays may differ by a little, but the window lands on the right display).
//!
//! The display is chosen by [`crate::dialog_placement::monitor_of`], the rule dialogs follow too (ADR 0054), so a
//! dialog and the browser started from the same window open on the same display.

use chromium_driver::WindowBounds;

pub use crate::dialog_placement::{Monitor as Display, Rect};

/// The browser window's size when the display has room (logical px).
pub const WANTED: (f64, f64) = (1100.0, 800.0);
/// The most of the display's work area it takes.
const SHARE: f64 = 0.9;

/// Where the browser window opens for an owner window at `owner`; `None` when no display is known (the browser then
/// chooses, as before).
pub fn place(owner: &Rect, displays: &[Display]) -> Option<WindowBounds> {
    let d = displays[crate::dialog_placement::monitor_of(owner, displays)?];
    let s = d.scale();
    let (wx, wy, ww, wh) = (d.work.x / s, d.work.y / s, d.work.width / s, d.work.height / s);
    let w = WANTED.0.min(ww * SHARE).floor();
    let h = WANTED.1.min(wh * SHARE).floor();
    // Centred on the owner (clamped to the display), else on the display when the owner is elsewhere.
    let (cx, cy) = if owner.is_usable() {
        ((owner.x + owner.width / 2.0) / s, (owner.y + owner.height / 2.0) / s)
    } else {
        (wx + ww / 2.0, wy + wh / 2.0)
    };
    let clamp = |v: f64, lo: f64, hi: f64| v.max(lo).min(hi.max(lo));
    let x = clamp(cx - w / 2.0, wx, wx + ww - w);
    let y = clamp(cy - h / 2.0, wy, wy + wh - h);
    Some(WindowBounds {
        x: x.round() as i32,
        y: y.round() as i32,
        width: w as u32,
        height: h as u32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x: f64, y: f64, width: f64, height: f64) -> Rect {
        Rect { x, y, width, height }
    }

    fn b(x: i32, y: i32, width: u32, height: u32) -> WindowBounds {
        WindowBounds { x, y, width, height }
    }

    /// A Retina laptop (2×, 1512×945 points, menu bar 37 points) and a 1× display of 1920×1080 left of it.
    fn laptop_and_left_monitor() -> Vec<Display> {
        vec![
            Display {
                work: r(0.0, 74.0, 3024.0, 1816.0),
                scale: 2.0,
            },
            Display {
                work: r(-1920.0, 0.0, 1920.0, 1040.0),
                scale: 1.0,
            },
        ]
    }

    #[test]
    fn the_browser_opens_on_the_display_of_the_window_that_started_it() {
        let displays = laptop_and_left_monitor();
        let cases = [
            // (owner window, physical px) → browser bounds (display units)
            (
                "on the laptop, centred",
                r(512.0, 274.0, 2000.0, 1400.0),
                Some(b(206, 87, 1100, 800)),
            ),
            (
                "on the left display (negative x)",
                r(-1700.0, 100.0, 1200.0, 800.0),
                Some(b(-1650, 100, 1100, 800)),
            ),
            (
                "mostly on the left display",
                r(-1500.0, 200.0, 1600.0, 900.0),
                Some(b(-1250, 240, 1100, 800)),
            ),
            (
                "off every display: the nearest",
                r(-4000.0, 300.0, 800.0, 600.0),
                Some(b(-1920, 200, 1100, 800)),
            ),
            (
                "at the laptop's far corner: kept inside",
                r(2900.0, 1800.0, 400.0, 300.0),
                Some(b(412, 145, 1100, 800)),
            ),
        ];
        for (what, owner, want) in cases {
            assert_eq!(place(&owner, &displays), want, "{what}");
        }
    }

    #[test]
    fn mixed_scale_uses_the_scale_of_the_chosen_display() {
        // A 1.5× display above a 1× one (negative y).
        let displays = [
            Display {
                work: r(0.0, -2160.0, 3840.0, 2100.0),
                scale: 1.5,
            },
            Display {
                work: r(0.0, 0.0, 1920.0, 1040.0),
                scale: 1.0,
            },
        ];
        let p = place(&r(1000.0, -1500.0, 1200.0, 900.0), &displays).unwrap();
        assert_eq!(p, b(517, -1100, 1100, 800));
        // In the display's own units the window fits its work area.
        let (wx, wy) = (0.0 / 1.5, -2160.0 / 1.5);
        assert!(f64::from(p.x) >= wx && f64::from(p.y) >= wy && f64::from(p.y + 800) <= wy + 2100.0 / 1.5);
    }

    #[test]
    fn a_small_display_gets_a_smaller_window_and_nonsense_is_survived() {
        let small = [Display {
            work: r(0.0, 0.0, 1024.0, 700.0),
            scale: 0.0, // nonsense: taken as 1
        }];
        assert_eq!(place(&r(0.0, 0.0, 1024.0, 700.0), &small), Some(b(52, 35, 921, 630)));
        assert_eq!(place(&r(0.0, 0.0, 10.0, 10.0), &[]), None, "no display known: the browser chooses");
        let broken = [Display {
            work: r(f64::NAN, 0.0, 100.0, 100.0),
            scale: 1.0,
        }];
        assert_eq!(place(&r(0.0, 0.0, 10.0, 10.0), &broken), None);
        // An owner with no usable size is centred on its display.
        let displays = laptop_and_left_monitor();
        assert_eq!(place(&r(-100.0, 10.0, 0.0, 0.0), &displays), Some(b(-1510, 120, 1100, 800)));
    }
}
