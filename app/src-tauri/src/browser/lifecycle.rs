//! The life of a window's AI browser, owned by [`super::sessions::Renderers`] alone:
//!
//! ```text
//!   none ──launch──▶ launching ──started──▶ ready ──close/finish/stop──▶ closing ──ended──▶ gone(ended)
//!                        │                    │
//!                        └─failed─▶ gone(launchFailed)
//!                                             └─window closed / quit / crash / lost connection─▶ gone(reason)
//!   gone ──launch──▶ launching …
//! ```
//!
//! Whatever ends a browser (Debrief, or the user closing its window or quitting it) moves it to `gone`: its process is
//! ended, its throwaway profile removed and its "kept open" mark dropped, so the next research starts a fresh browser.
//! Nothing here depends on Tauri; the phase is reported to the window (`lifecycle` events) and can be read for health.

use serde::Serialize;

/// Why a browser is gone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum GoneReason {
    /// The user closed the browser window (the browser stayed running without one and was ended).
    WindowClosed,
    /// The browser process ended by itself: the user quit it, or it crashed.
    Exited,
    /// The DevTools connection to the browser ended.
    Disconnected,
    /// Debrief ended it: research finished, Close or Stop, its Debrief window closed, or Debrief quit.
    Ended,
    /// It could not be started.
    LaunchFailed,
}

impl From<chromium_driver::Gone> for GoneReason {
    fn from(g: chromium_driver::Gone) -> Self {
        match g {
            chromium_driver::Gone::Exited => GoneReason::Exited,
            chromium_driver::Gone::Disconnected => GoneReason::Disconnected,
            chromium_driver::Gone::PageClosed => GoneReason::WindowClosed,
        }
    }
}

/// Where a window's browser is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Phase {
    /// No browser was started for the window yet.
    None,
    Launching,
    Ready,
    Closing,
    Gone {
        reason: GoneReason,
    },
}

impl Phase {
    /// Whether `self → to` is a step of the life cycle (anything else is a bug in the one owner of the phase).
    pub fn may_become(self, to: Phase) -> bool {
        use Phase::*;
        matches!(
            (self, to),
            (None | Gone { .. }, Launching)
                | (Launching, Ready | Closing)
                | (
                    Launching,
                    Gone {
                        reason: GoneReason::LaunchFailed
                    }
                )
                | (Ready, Ready | Closing | Gone { .. })
                | (Closing, Gone { reason: GoneReason::Ended })
        )
    }
}

/// A window's browser as reported: its phase, and whether research left it open for the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Lifecycle {
    #[serde(flatten)]
    pub phase: Phase,
    /// Research ended but the window stays open because the user took part in it (only while `ready`).
    pub kept: bool,
}

impl Lifecycle {
    pub const NONE: Lifecycle = Lifecycle {
        phase: Phase::None,
        kept: false,
    };
}

/// What every window's browser life cycle means for the `browser` service (ADR 0055): a start under way is
/// `starting`; else a window whose last start failed is down (fixed in Settings ▸ Web browser); anything else (none,
/// ready, closing, or ended by the user or Debrief) is fine.
pub fn service_of(all: &[(String, Lifecycle)]) -> crate::health::hub::BrowserService {
    use crate::health::hub::BrowserService;
    if all.iter().any(|(_, l)| l.phase == Phase::Launching) {
        return BrowserService::Starting;
    }
    let failed = Phase::Gone {
        reason: GoneReason::LaunchFailed,
    };
    if all.iter().any(|(_, l)| l.phase == failed) {
        return BrowserService::LaunchFailed;
    }
    BrowserService::Idle
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const GONE: Phase = Phase::Gone {
        reason: GoneReason::WindowClosed,
    };
    const ENDED: Phase = Phase::Gone { reason: GoneReason::Ended };

    #[test]
    fn the_life_cycle_allows_only_its_steps() {
        use Phase::*;
        for (from, to) in [
            (None, Launching),
            (Launching, Ready),
            (Launching, Closing),
            (
                Launching,
                Gone {
                    reason: GoneReason::LaunchFailed,
                },
            ),
            (Ready, Ready),
            (Ready, Closing),
            (Ready, GONE),
            (Closing, ENDED),
            (GONE, Launching),
            (ENDED, Launching),
        ] {
            assert!(from.may_become(to), "{from:?} → {to:?}");
        }
        for (from, to) in [
            (None, Ready),
            (GONE, Ready),
            (Closing, Ready),
            (Closing, Launching),
            (Ready, Launching),
            (Launching, GONE),
            (Closing, GONE),
            (None, Closing),
        ] {
            assert!(!from.may_become(to), "{from:?} → {to:?}");
        }
    }

    #[test]
    fn the_browser_service_follows_the_windows_life_cycles() {
        use crate::health::hub::BrowserService::*;
        let one = |phase: Phase| vec![("main".to_string(), Lifecycle { phase, kept: false })];
        assert_eq!(service_of(&[]), Idle);
        assert_eq!(service_of(&one(Phase::None)), Idle);
        assert_eq!(service_of(&one(Phase::Launching)), Starting);
        assert_eq!(service_of(&one(Phase::Ready)), Idle);
        assert_eq!(service_of(&one(Phase::Closing)), Idle);
        for reason in [
            GoneReason::WindowClosed,
            GoneReason::Exited,
            GoneReason::Disconnected,
            GoneReason::Ended,
        ] {
            assert_eq!(
                service_of(&one(Phase::Gone { reason })),
                Idle,
                "{reason:?}: the user or Debrief ended it"
            );
        }
        let failed = Phase::Gone {
            reason: GoneReason::LaunchFailed,
        };
        assert_eq!(service_of(&one(failed)), LaunchFailed);
        // Another window starting its browser is a new attempt: starting, not down.
        let mut both = one(failed);
        both.push((
            "w-2".into(),
            Lifecycle {
                phase: Phase::Launching,
                kept: false,
            },
        ));
        assert_eq!(service_of(&both), Starting);
    }

    #[test]
    fn driver_ends_map_to_reasons() {
        assert_eq!(GoneReason::from(chromium_driver::Gone::PageClosed), GoneReason::WindowClosed);
        assert_eq!(GoneReason::from(chromium_driver::Gone::Exited), GoneReason::Exited);
        assert_eq!(GoneReason::from(chromium_driver::Gone::Disconnected), GoneReason::Disconnected);
    }

    #[test]
    fn the_wire_shape_is_flat() {
        let l = Lifecycle { phase: GONE, kept: false };
        assert_eq!(
            serde_json::to_value(l).unwrap(),
            json!({"state": "gone", "reason": "windowClosed", "kept": false})
        );
        assert_eq!(
            serde_json::to_value(Lifecycle {
                phase: Phase::Ready,
                kept: true
            })
            .unwrap(),
            json!({"state": "ready", "kept": true})
        );
    }
}
