//! How the assistant reaches the workspace. A chat turn or a use case can take minutes (the model is slow),
//! while the app must keep importing and tailing files into the same store. So the assistant never holds the
//! store across generation: every tool execution and every validation step borrows it briefly through
//! [`StoreAccess`], and the app implements that port with a short read lock per call.

use imt_core::LogStore;

/// The port to the workspace. Contract: `visit` calls `f` exactly once, with a store that stays valid for that
/// call only; it may block briefly (a lock), and must never be called re-entrantly from inside `f`.
pub trait StoreAccess {
    fn visit(&self, f: &mut dyn FnMut(&dyn LogStore));
}

/// A store, borrowed for the whole lifetime of the caller (tests, tools) or reached through a [`StoreAccess`]
/// port (the app). `&dyn LogStore` and `&dyn StoreAccess` both convert into it, so existing call sites that
/// pass a store keep working.
#[derive(Clone, Copy)]
pub enum StoreRef<'a> {
    Direct(&'a dyn LogStore),
    Access(&'a dyn StoreAccess),
}

impl<'a> From<&'a dyn LogStore> for StoreRef<'a> {
    fn from(s: &'a dyn LogStore) -> Self {
        StoreRef::Direct(s)
    }
}

impl<'a> From<&'a dyn StoreAccess> for StoreRef<'a> {
    fn from(a: &'a dyn StoreAccess) -> Self {
        StoreRef::Access(a)
    }
}

impl StoreRef<'_> {
    /// Runs `f` with the store, holding it only for this call. Never nest `with` calls.
    pub fn with<R>(&self, f: impl FnOnce(&dyn LogStore) -> R) -> R {
        match self {
            StoreRef::Direct(s) => f(*s),
            StoreRef::Access(a) => {
                let mut f = Some(f);
                let mut out = None;
                a.visit(&mut |s| {
                    if let Some(f) = f.take() {
                        out = Some(f(s));
                    }
                });
                out.unwrap_or_else(|| unreachable!("StoreAccess::visit must call back exactly once"))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use imt_core::{Profile, Workspace};
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Counting(Workspace, AtomicUsize);
    impl StoreAccess for Counting {
        fn visit(&self, f: &mut dyn FnMut(&dyn LogStore)) {
            self.1.fetch_add(1, Ordering::SeqCst);
            f(&self.0)
        }
    }

    #[test]
    fn both_forms_reach_the_store() {
        let ws = Workspace::new(Profile::default_profile()).unwrap();
        let direct: StoreRef = (&ws as &dyn LogStore).into();
        assert_eq!(direct.with(|s| s.summary().total), 0);
        let c = Counting(ws, AtomicUsize::new(0));
        let access: StoreRef = (&c as &dyn StoreAccess).into();
        assert_eq!(access.with(|s| s.summary().total), 0);
        access.with(|_| ());
        assert_eq!(c.1.load(Ordering::SeqCst), 2);
    }
}
