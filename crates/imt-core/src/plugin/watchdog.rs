//! Watchdog for hung in-process plugin calls.
//!
//! Every guarded call with a time budget pushes a frame on its thread's call
//! stack (a cheap, uncontended lock). One background thread scans the stacks
//! and, when a frame outlives its budget, marks the plugin hung in its
//! [`Health`](crate::guard::Health): that counts a failure towards quarantine
//! and makes further calls of the plugin fail fast until the stuck call
//! returns. The stuck call itself cannot be pre-empted in process; only the
//! out-of-process plugin host (ADR 0002) can stop it. Callers that can afford
//! it use [`crate::guard::Guard::call_detached`] to stop waiting instead.

use crate::guard::{fmt_duration, CallKind, Guard};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};

/// Scan interval while calls are being watched, and while none are.
const TICK_BUSY: Duration = Duration::from_millis(10);
const TICK_IDLE: Duration = Duration::from_millis(100);

struct Frame {
    id: u64,
    guard: Guard,
    kind: CallKind,
    started: Instant,
    budget: Duration,
    flagged: bool,
}

type Stack = Arc<Mutex<Vec<Frame>>>;

struct Dog {
    stacks: Mutex<Vec<Weak<Mutex<Vec<Frame>>>>>,
}

static DOG: OnceLock<Option<&'static Dog>> = OnceLock::new();
static NEXT: AtomicU64 = AtomicU64::new(1);

thread_local! {
    static STACK: Option<Stack> = register();
}

fn dog() -> Option<&'static Dog> {
    *DOG.get_or_init(|| {
        let dog: &'static Dog = Box::leak(Box::new(Dog {
            stacks: Mutex::new(Vec::new()),
        }));
        // Without the thread, budgets are not enforced but calls still work.
        std::thread::Builder::new()
            .name("imt-plugin-watchdog".into())
            .spawn(move || loop {
                let busy = scan(dog);
                std::thread::sleep(if busy { TICK_BUSY } else { TICK_IDLE });
            })
            .ok()
            .map(|_| dog)
    })
}

fn register() -> Option<Stack> {
    let dog = dog()?;
    let stack: Stack = Arc::new(Mutex::new(Vec::new()));
    dog.stacks.lock().unwrap_or_else(|p| p.into_inner()).push(Arc::downgrade(&stack));
    Some(stack)
}

/// Flags overdue frames and marks their plugins hung; true while any call is watched.
fn scan(dog: &Dog) -> bool {
    let mut overdue = Vec::new();
    let mut busy = false;
    {
        let mut stacks = dog.stacks.lock().unwrap_or_else(|p| p.into_inner());
        stacks.retain(|w| w.strong_count() > 0);
        for stack in stacks.iter().filter_map(Weak::upgrade) {
            let mut frames = stack.lock().unwrap_or_else(|p| p.into_inner());
            busy |= !frames.is_empty();
            for f in frames.iter_mut().filter(|f| !f.flagged && f.started.elapsed() > f.budget) {
                f.flagged = true;
                overdue.push((f.guard.clone(), f.kind, f.budget));
            }
        }
    }
    for (guard, kind, budget) in overdue {
        let message = format!(
            "plugin \"{}\" ({}) did not finish within {} and is considered hung",
            guard.owner(),
            kind.label(),
            fmt_duration(budget)
        );
        guard.health().mark_hung(guard.owner(), &message);
    }
    busy
}

/// Removes its frame when the call returns, clearing the hung mark it caused.
pub(crate) struct Watched {
    id: u64,
    stack: Option<Stack>,
}

impl Drop for Watched {
    fn drop(&mut self) {
        let Some(stack) = &self.stack else { return };
        let frame = {
            let mut frames = stack.lock().unwrap_or_else(|p| p.into_inner());
            frames.iter().rposition(|f| f.id == self.id).map(|i| frames.remove(i))
        };
        if let Some(f) = frame.filter(|f| f.flagged) {
            f.guard.health().unhang(f.guard.owner());
        }
    }
}

/// Watches the current call of `guard` against `budget`.
pub(crate) fn enter(guard: &Guard, kind: CallKind, budget: Duration) -> Watched {
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let stack = STACK.try_with(|s| s.clone()).ok().flatten();
    if let Some(s) = &stack {
        s.lock().unwrap_or_else(|p| p.into_inner()).push(Frame {
            id,
            guard: guard.clone(),
            kind,
            started: Instant::now(),
            budget,
            flagged: false,
        });
    }
    Watched { id, stack }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::guard::Health;

    #[test]
    fn overdue_calls_are_flagged_once_and_cleared_on_return() {
        let health = Arc::new(Health::default());
        let guard = Guard::new("t.sleepy", health.clone());
        let w = enter(&guard, CallKind::Service, Duration::from_millis(20));
        let deadline = Instant::now() + Duration::from_secs(5);
        while !health.get("t.sleepy").hung && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        let h = health.get("t.sleepy");
        assert!(h.hung && h.failures == 1, "{h:?}");
        assert!(h.last_error.unwrap().contains("(service) did not finish within 20 ms"));
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(health.get("t.sleepy").failures, 1, "flagged once");
        drop(w);
        assert!(!health.get("t.sleepy").hung);
        // A frame that returns in time leaves no trace.
        drop(enter(&guard, CallKind::Service, Duration::from_secs(60)));
        assert_eq!(health.get("t.sleepy").failures, 1);
    }
}
