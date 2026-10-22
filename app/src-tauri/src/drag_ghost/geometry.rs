//! Where the drag ghost goes: pure geometry in physical pixels, so it works across monitors with different scale
//! factors (logical coordinates of two monitors with different DPI do not line up). The pointer holds the ghost by
//! its title bar; the ghost stays fully inside the work area of the monitor under the pointer, at that monitor's scale.

use crate::panel_windows::Bounds;

/// A rectangle in physical screen pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhysRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl PhysRect {
    fn right(&self) -> f64 {
        f64::from(self.x) + f64::from(self.width)
    }

    fn bottom(&self) -> f64 {
        f64::from(self.y) + f64::from(self.height)
    }

    fn contains(&self, x: f64, y: f64) -> bool {
        x >= f64::from(self.x) && x < self.right() && y >= f64::from(self.y) && y < self.bottom()
    }

    /// The squared distance from a point to this rectangle (0 inside).
    fn distance2(&self, x: f64, y: f64) -> f64 {
        let dx = (f64::from(self.x) - x).max(x - self.right()).max(0.0);
        let dy = (f64::from(self.y) - y).max(y - self.bottom()).max(0.0);
        dx * dx + dy * dy
    }

    fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }
}

/// A monitor: its bounds, its usable area (without task bar, dock or menu bar) and its scale factor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Screen {
    pub bounds: PhysRect,
    pub work: PhysRect,
    pub scale: f64,
}

impl Screen {
    /// The scale factor, or 1 when the platform reported nonsense.
    pub fn scale(&self) -> f64 {
        if self.scale.is_finite() && self.scale > 0.0 {
            self.scale
        } else {
            1.0
        }
    }

    /// Where a window may go on this monitor: the work area, or the whole monitor when the work area is unknown.
    fn usable(&self) -> PhysRect {
        if self.work.is_empty() {
            self.bounds
        } else {
            self.work
        }
    }
}

/// The ghost's size in logical pixels and the point (from its top-left corner, logical px) the pointer holds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GhostSize {
    pub width: f64,
    pub height: f64,
    pub grab_x: f64,
    pub grab_y: f64,
}

/// Where the ghost is: its physical rectangle and the scale factor of its monitor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    pub rect: PhysRect,
    pub scale: f64,
}

impl Placement {
    /// The same place in logical pixels of its monitor, as a pop-out window is positioned (each monitor's logical
    /// coordinates are its physical ones divided by its own scale).
    pub fn logical(&self) -> Bounds {
        let s = if self.scale.is_finite() && self.scale > 0.0 {
            self.scale
        } else {
            1.0
        };
        Bounds {
            x: f64::from(self.rect.x) / s,
            y: f64::from(self.rect.y) / s,
            width: f64::from(self.rect.width) / s,
            height: f64::from(self.rect.height) / s,
        }
    }
}

/// The monitor under the cursor (physical px), or the nearest one when the cursor is between monitors.
pub fn screen_at(cursor: (f64, f64), screens: &[Screen]) -> Option<&Screen> {
    let (x, y) = cursor;
    if !x.is_finite() || !y.is_finite() {
        return screens.first();
    }
    screens.iter().find(|s| s.bounds.contains(x, y)).or_else(|| {
        screens
            .iter()
            .min_by(|a, b| a.bounds.distance2(x, y).total_cmp(&b.bounds.distance2(x, y)))
    })
}

fn clamp(v: f64, lo: f64, hi: f64) -> f64 {
    v.max(lo).min(hi.max(lo))
}

/// The ghost for a cursor (physical px) on `screen`: its title bar under the pointer at the grab point, scaled to
/// the monitor, no larger than its work area and moved inside it.
pub fn ghost_rect(cursor: (f64, f64), size: GhostSize, screen: &Screen) -> PhysRect {
    let s = screen.scale();
    let area = screen.usable();
    let width = (size.width * s).round().clamp(1.0, f64::from(area.width.max(1)));
    let height = (size.height * s).round().clamp(1.0, f64::from(area.height.max(1)));
    let (cx, cy) = if cursor.0.is_finite() && cursor.1.is_finite() {
        cursor
    } else {
        (f64::from(area.x), f64::from(area.y))
    };
    let x = clamp((cx - size.grab_x * s).round(), f64::from(area.x), area.right() - width);
    let y = clamp((cy - size.grab_y * s).round(), f64::from(area.y), area.bottom() - height);
    // In range: the work area's own coordinates bound x and y, and its size bounds the width and height.
    PhysRect {
        x: x as i32,
        y: y as i32,
        width: width as u32,
        height: height as u32,
    }
}

/// Where the ghost goes for a cursor: on the monitor under it. `None` without monitors.
pub fn place(cursor: (f64, f64), size: GhostSize, screens: &[Screen]) -> Option<Placement> {
    let screen = screen_at(cursor, screens)?;
    Some(Placement {
        rect: ghost_rect(cursor, size, screen),
        scale: screen.scale(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, width: u32, height: u32) -> PhysRect {
        PhysRect { x, y, width, height }
    }

    /// A 1920×1080 monitor at 100% with a 40 px task bar, and a 4K one at 200% to its right.
    fn screens() -> Vec<Screen> {
        vec![
            Screen {
                bounds: rect(0, 0, 1920, 1080),
                work: rect(0, 0, 1920, 1040),
                scale: 1.0,
            },
            Screen {
                bounds: rect(1920, 0, 3840, 2160),
                work: rect(1920, 0, 3840, 2080),
                scale: 2.0,
            },
        ]
    }

    const SIZE: GhostSize = GhostSize {
        width: 560.0,
        height: 420.0,
        grab_x: 120.0,
        grab_y: 12.0,
    };

    #[test]
    fn the_title_bar_sits_under_the_pointer() {
        let p = place((500.0, 300.0), SIZE, &screens()).unwrap();
        assert_eq!(p.rect, rect(380, 288, 560, 420));
        assert_eq!(p.scale, 1.0);
        assert_eq!(
            p.logical(),
            Bounds {
                x: 380.0,
                y: 288.0,
                width: 560.0,
                height: 420.0
            }
        );
    }

    #[test]
    fn on_a_high_dpi_monitor_it_is_scaled_and_reported_in_that_monitor_s_logical_pixels() {
        let p = place((3000.0, 1000.0), SIZE, &screens()).unwrap();
        assert_eq!(p.scale, 2.0);
        assert_eq!(p.rect, rect(3000 - 240, 1000 - 24, 1120, 840));
        let b = p.logical();
        assert_eq!((b.x, b.y, b.width, b.height), (1380.0, 488.0, 560.0, 420.0));
    }

    #[test]
    fn it_stays_inside_the_work_area_of_the_monitor_under_the_pointer() {
        let s = screens();
        // Near the top-left corner: pushed right and down.
        assert_eq!(place((10.0, 2.0), SIZE, &s).unwrap().rect, rect(0, 0, 560, 420));
        // Over the task bar: above it.
        assert_eq!(place((900.0, 1070.0), SIZE, &s).unwrap().rect, rect(780, 1040 - 420, 560, 420));
        // At the right edge of the first monitor: it stays on the first monitor, not across the seam.
        assert_eq!(place((1910.0, 500.0), SIZE, &s).unwrap().rect.x, 1920 - 560);
        // At the far right of the second: inside it.
        let r = place((5759.0, 10.0), SIZE, &s).unwrap().rect;
        assert_eq!(r.x + r.width as i32, 1920 + 3840);
    }

    #[test]
    fn larger_than_the_monitor_it_shrinks_to_fit() {
        let small = [Screen {
            bounds: rect(0, 0, 800, 600),
            work: rect(0, 0, 800, 560),
            scale: 1.0,
        }];
        let big = GhostSize {
            width: 2000.0,
            height: 2000.0,
            ..SIZE
        };
        assert_eq!(place((400.0, 300.0), big, &small).unwrap().rect, rect(0, 0, 800, 560));
    }

    #[test]
    fn monitors_left_of_and_above_the_primary_have_negative_coordinates() {
        let s = [
            Screen {
                bounds: rect(-1280, -200, 1280, 1024),
                work: rect(-1280, -200, 1280, 1024),
                scale: 1.25,
            },
            screens()[0],
        ];
        let p = place((-640.0, 100.0), SIZE, &s).unwrap();
        assert_eq!(p.scale, 1.25);
        assert_eq!(p.rect, rect(-640 - 150, 100 - 15, 700, 525));
        assert_eq!(p.logical().x, -632.0);
    }

    #[test]
    fn between_monitors_the_nearest_one_takes_it() {
        let s = [
            Screen {
                bounds: rect(0, 0, 1000, 800),
                work: rect(0, 0, 1000, 800),
                scale: 1.0,
            },
            Screen {
                bounds: rect(1200, 0, 1000, 800),
                work: rect(1200, 0, 1000, 800),
                scale: 1.0,
            },
        ];
        assert_eq!(screen_at((1150.0, 10.0), &s).unwrap().bounds.x, 1200);
        assert_eq!(screen_at((1050.0, 10.0), &s).unwrap().bounds.x, 0);
        assert_eq!(screen_at((500.0, 5000.0), &s).unwrap().bounds.x, 0);
    }

    #[test]
    fn nonsense_from_the_platform_never_panics() {
        assert!(place((1.0, 1.0), SIZE, &[]).is_none());
        let odd = [Screen {
            bounds: rect(0, 0, 0, 0),
            work: rect(0, 0, 0, 0),
            scale: f64::NAN,
        }];
        let p = place((f64::NAN, f64::INFINITY), SIZE, &odd).unwrap();
        assert_eq!(p.scale, 1.0);
        assert_eq!(p.rect, rect(0, 0, 1, 1));
        assert_eq!(screen_at((f64::NAN, 0.0), &screens()).unwrap().bounds.x, 0);
        let zero = Placement {
            rect: rect(10, 10, 10, 10),
            scale: 0.0,
        };
        assert_eq!(zero.logical().x, 10.0);
        // A missing work area: the whole monitor is used.
        let no_work = [Screen {
            bounds: rect(0, 0, 1000, 800),
            work: rect(0, 0, 0, 0),
            scale: 1.0,
        }];
        assert_eq!(place((999.0, 799.0), SIZE, &no_work).unwrap().rect, rect(440, 380, 560, 420));
    }
}
