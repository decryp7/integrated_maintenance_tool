//! Inter-plugin services and plugin-defined extension points.
//!
//! A provider registers an `Arc<dyn Trait>` under a service id and semver
//! version; consumers look it up by id, trait type (`TypeId`) and version
//! requirement. A plugin may also declare an extension point (an id and a
//! trait) that other plugins contribute implementations to.
//!
//! Consumers never get the bare `Arc`: they get a [`Service`] handle whose
//! [`Service::call`] runs their closure under the provider's guard, so a
//! panicking provider becomes an error counted against the provider, and a
//! quarantined, hung or inactive provider is refused.

use crate::guard::{CallKind, Guard};
use anyhow::{anyhow, Result};
use semver::{Version, VersionReq};
use std::any::{Any, TypeId};
use std::sync::Arc;
use std::time::Duration;

/// Plugin id recorded for services and points registered outside a plugin.
pub const HOST: &str = "host";

/// A guarded handle to a service or an extension-point contribution.
pub struct Service<T: ?Sized> {
    id: String,
    version: Version,
    provider: String,
    inner: Arc<T>,
    guard: Option<Guard>,
}

impl<T: ?Sized> Clone for Service<T> {
    fn clone(&self) -> Self {
        Service {
            id: self.id.clone(),
            version: self.version.clone(),
            provider: self.provider.clone(),
            inner: self.inner.clone(),
            guard: self.guard.clone(),
        }
    }
}

impl<T: ?Sized> std::fmt::Debug for Service<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Service")
            .field("id", &self.id)
            .field("version", &self.version)
            .field("provider", &self.provider)
            .finish()
    }
}

impl<T: ?Sized + Send + Sync + 'static> Service<T> {
    /// The service id (or extension point id for a contribution).
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn version(&self) -> &Version {
        &self.version
    }

    /// Id of the plugin that provides it.
    pub fn provider(&self) -> &str {
        &self.provider
    }

    fn context(&self, e: anyhow::Error) -> anyhow::Error {
        if self.guard.as_ref().is_some_and(|g| g.health().is_quarantined(&self.provider)) {
            return anyhow!("service \"{}\": provider \"{}\" is quarantined", self.id, self.provider);
        }
        anyhow!("service \"{}\" (provider \"{}\"): {e:#}", self.id, self.provider)
    }

    /// Calls the service under the provider's guard and time budget.
    pub fn call<R>(&self, f: impl FnOnce(&T) -> R) -> Result<R> {
        self.try_call(|t| Ok(f(t)))
    }

    /// [`Service::call`] for closures that return a `Result` themselves.
    pub fn try_call<R>(&self, f: impl FnOnce(&T) -> Result<R>) -> Result<R> {
        match &self.guard {
            Some(g) => g.call(CallKind::Service, &self.id, || f(&self.inner)).map_err(|e| self.context(e)),
            None => f(&self.inner),
        }
    }

    /// Calls the service on a helper thread and abandons the call when it
    /// exceeds the service budget (the provider is then marked hung).
    pub fn call_detached<R: Send + 'static>(&self, f: impl FnOnce(&T) -> R + Send + 'static) -> Result<R> {
        let inner = self.inner.clone();
        match &self.guard {
            Some(g) => g
                .call_detached(CallKind::Service, &self.id, move || Ok(f(&inner)))
                .map_err(|e| self.context(e)),
            None => Ok(f(&inner)),
        }
    }

    /// [`Service::call_detached`] with its own time budget instead of the
    /// service budget, for calls known to take long (importing a large file).
    /// Past `budget` the call is abandoned and the provider marked hung.
    pub fn call_detached_within<R: Send + 'static>(&self, budget: Duration, f: impl FnOnce(&T) -> R + Send + 'static) -> Result<R> {
        self.try_call_detached_within(budget, move |t| Ok(f(t)))
    }

    /// [`Service::call_detached_within`] for closures that return a `Result` themselves.
    pub fn try_call_detached_within<R: Send + 'static>(
        &self,
        budget: Duration,
        f: impl FnOnce(&T) -> Result<R> + Send + 'static,
    ) -> Result<R> {
        let inner = self.inner.clone();
        match &self.guard {
            Some(g) => g
                .call_detached_within(CallKind::Service, Some(budget), &self.id, move || f(&inner))
                .map_err(|e| self.context(e)),
            None => f(&inner),
        }
    }
}

struct Entry {
    id: String,
    version: Version,
    owner: String,
    guard: Option<Guard>,
    type_id: TypeId,
    type_name: &'static str,
    /// An `Arc<T>`.
    value: Box<dyn Any + Send + Sync>,
}

impl Entry {
    fn handle<T: ?Sized + Send + Sync + 'static>(&self) -> Option<Service<T>> {
        let inner = self.value.downcast_ref::<Arc<T>>()?.clone();
        Some(Service {
            id: self.id.clone(),
            version: self.version.clone(),
            provider: self.owner.clone(),
            inner,
            guard: self.guard.clone(),
        })
    }
}

struct Point {
    id: String,
    owner: String,
    type_id: TypeId,
    type_name: &'static str,
}

/// Services, extension points and contributions of every installed plugin.
#[derive(Default)]
pub(crate) struct ServiceTable {
    services: Vec<Entry>,
    points: Vec<Point>,
    contributions: Vec<Entry>,
}

/// Whether a plugin may be called right now; `Err` holds why not.
pub(crate) type Availability<'a> = &'a dyn Fn(&str) -> std::result::Result<(), String>;

impl ServiceTable {
    pub fn provide<T: ?Sized + Send + Sync + 'static>(
        &mut self,
        owner: &str,
        guard: Option<Guard>,
        id: &str,
        version: Version,
        service: Arc<T>,
    ) -> Result<()> {
        if let Some(e) = self.services.iter().find(|e| e.id == id) {
            return Err(anyhow!("service \"{id}\" is already provided by plugin \"{}\"", e.owner));
        }
        self.services.push(Entry {
            id: id.into(),
            version,
            owner: owner.into(),
            guard,
            type_id: TypeId::of::<T>(),
            type_name: std::any::type_name::<T>(),
            value: Box::new(service),
        });
        Ok(())
    }

    /// Typed lookup by id and version requirement; the provider must be available.
    pub fn service<T: ?Sized + Send + Sync + 'static>(&self, id: &str, req: &VersionReq, available: Availability) -> Result<Service<T>> {
        let e = self
            .services
            .iter()
            .find(|e| e.id == id)
            .ok_or_else(|| anyhow!("no installed plugin provides service \"{id}\""))?;
        if !req.matches(&e.version) {
            return Err(anyhow!(
                "service \"{id}\" {req} is not available: plugin \"{}\" provides version {}",
                e.owner,
                e.version
            ));
        }
        if e.type_id != TypeId::of::<T>() {
            return Err(anyhow!(
                "service \"{id}\" of plugin \"{}\" implements {}, not the requested {}",
                e.owner,
                e.type_name,
                std::any::type_name::<T>()
            ));
        }
        available(&e.owner).map_err(|why| anyhow!("service \"{id}\" is not available: {why}"))?;
        e.handle().ok_or_else(|| anyhow!("service \"{id}\": internal type mismatch"))
    }

    pub fn declare<T: ?Sized + 'static>(&mut self, owner: &str, id: &str) -> Result<()> {
        if let Some(p) = self.points.iter().find(|p| p.id == id) {
            return Err(anyhow!("extension point \"{id}\" is already declared by plugin \"{}\"", p.owner));
        }
        self.points.push(Point {
            id: id.into(),
            owner: owner.into(),
            type_id: TypeId::of::<T>(),
            type_name: std::any::type_name::<T>(),
        });
        Ok(())
    }

    pub fn contribute<T: ?Sized + Send + Sync + 'static>(
        &mut self,
        owner: &str,
        guard: Option<Guard>,
        version: Version,
        id: &str,
        item: Arc<T>,
    ) {
        self.contributions.push(Entry {
            id: id.into(),
            version,
            owner: owner.into(),
            guard,
            type_id: TypeId::of::<T>(),
            type_name: std::any::type_name::<T>(),
            value: Box::new(item),
        });
    }

    /// Contributions to a declared point of type `T` from available plugins, in install order.
    pub fn contributions<T: ?Sized + Send + Sync + 'static>(&self, id: &str, available: Availability) -> Vec<Service<T>> {
        let Some(point) = self.points.iter().find(|p| p.id == id) else {
            return Vec::new();
        };
        if point.type_id != TypeId::of::<T>() {
            return Vec::new();
        }
        self.contributions
            .iter()
            .filter(|c| c.id == id && c.type_id == point.type_id && available(&c.owner).is_ok())
            .filter_map(Entry::handle)
            .collect()
    }

    /// Problems with `owner`'s contributions (undeclared point, wrong type).
    pub fn warnings(&self, owner: &str) -> Vec<String> {
        self.contributions
            .iter()
            .filter(|c| c.owner == owner)
            .filter_map(|c| match self.points.iter().find(|p| p.id == c.id) {
                None => Some(format!(
                    "contribution to extension point \"{}\" is inactive: no installed plugin declares it",
                    c.id
                )),
                Some(p) if p.type_id != c.type_id => Some(format!(
                    "contribution to extension point \"{}\" is inactive: it implements {}, the point (declared by \"{}\") expects {}",
                    c.id, c.type_name, p.owner, p.type_name
                )),
                Some(_) => None,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::guard::Health;

    trait Greeter: Send + Sync {
        fn greet(&self) -> String;
    }
    trait Other: Send + Sync {}
    struct Hello;
    impl Greeter for Hello {
        fn greet(&self) -> String {
            "hello".into()
        }
    }
    struct Boom;
    impl Greeter for Boom {
        fn greet(&self) -> String {
            panic!("greeter exploded")
        }
    }

    fn ok(_: &str) -> std::result::Result<(), String> {
        Ok(())
    }

    #[test]
    fn lookups_check_id_version_type_and_availability() {
        let health = Arc::new(Health::default());
        let mut t = ServiceTable::default();
        let g = Some(Guard::new("p", health.clone()));
        t.provide::<dyn Greeter>("p", g.clone(), "greet", Version::new(1, 2, 0), Arc::new(Hello))
            .unwrap();
        let e = t
            .provide::<dyn Greeter>("q", None, "greet", Version::new(1, 0, 0), Arc::new(Hello))
            .unwrap_err();
        assert!(e.to_string().contains("already provided by plugin \"p\""));
        let req = |s| VersionReq::parse(s).unwrap();

        let s = t.service::<dyn Greeter>("greet", &req("^1.1"), &ok).unwrap();
        assert_eq!((s.id(), s.provider(), s.version().to_string()), ("greet", "p", "1.2.0".into()));
        assert_eq!(s.call(|g| g.greet()).unwrap(), "hello");
        assert_eq!(s.call_detached(|g| g.greet()).unwrap(), "hello");
        assert!(format!("{s:?}").contains("greet"));
        let e = t.service::<dyn Greeter>("greet", &req("^2"), &ok).unwrap_err().to_string();
        assert_eq!(e, "service \"greet\" ^2 is not available: plugin \"p\" provides version 1.2.0");
        let e = t.service::<dyn Other>("greet", &req("*"), &ok).unwrap_err().to_string();
        assert!(e.contains("implements dyn") && e.contains("Greeter") && e.contains("Other"), "{e}");
        let e = t.service::<dyn Greeter>("nope", &req("*"), &ok).unwrap_err().to_string();
        assert!(e.contains("no installed plugin provides service \"nope\""));
        let down = |id: &str| Err(format!("plugin \"{id}\" is quarantined"));
        let e = t.service::<dyn Greeter>("greet", &req("*"), &down).unwrap_err().to_string();
        assert!(e.contains("is not available: plugin \"p\" is quarantined"));
    }

    #[test]
    fn panicking_providers_are_contained_and_quarantined() {
        let health = Arc::new(Health::default());
        let mut t = ServiceTable::default();
        t.provide::<dyn Greeter>(
            "p",
            Some(Guard::new("p", health.clone())),
            "g",
            Version::new(1, 0, 0),
            Arc::new(Boom),
        )
        .unwrap();
        let s = t.service::<dyn Greeter>("g", &VersionReq::STAR, &ok).unwrap();
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        for _ in 0..crate::guard::QUARANTINE_AFTER {
            let e = s.call(|g| g.greet()).unwrap_err().to_string();
            assert!(e.contains("greeter exploded") || e.contains("quarantined"), "{e}");
        }
        std::panic::set_hook(prev);
        assert!(health.is_quarantined("p"));
        assert_eq!(
            s.call(|g| g.greet()).unwrap_err().to_string(),
            "service \"g\": provider \"p\" is quarantined"
        );
        assert!(s.call_detached(|g| g.greet()).unwrap_err().to_string().contains("quarantined"));
        // Services of the host are not guarded.
        let mut t = ServiceTable::default();
        t.provide::<dyn Greeter>(HOST, None, "h", Version::new(1, 0, 0), Arc::new(Hello))
            .unwrap();
        let s = t.service::<dyn Greeter>("h", &VersionReq::STAR, &ok).unwrap();
        assert_eq!(s.clone().call_detached(|g| g.greet()).unwrap(), "hello");
        assert_eq!(s.try_call(|g| Ok(g.greet())).unwrap(), "hello");
    }

    #[test]
    fn contributions_need_a_declared_point_of_the_same_type() {
        let mut t = ServiceTable::default();
        let v = Version::new(1, 0, 0);
        t.contribute::<dyn Greeter>("a", None, v.clone(), "greeters", Arc::new(Hello));
        t.contribute::<dyn Greeter>("b", None, v.clone(), "nowhere", Arc::new(Hello));
        t.contribute::<dyn Other>("c", None, v.clone(), "greeters", Arc::new(OtherImpl));
        assert!(t.contributions::<dyn Greeter>("greeters", &ok).is_empty());
        t.declare::<dyn Greeter>("host", "greeters").unwrap();
        assert!(t
            .declare::<dyn Greeter>("x", "greeters")
            .unwrap_err()
            .to_string()
            .contains("already declared"));
        let all = t.contributions::<dyn Greeter>("greeters", &ok);
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].provider(), "a");
        assert!(t.contributions::<dyn Other>("greeters", &ok).is_empty());
        let none = |_: &str| Err("down".to_string());
        assert!(t.contributions::<dyn Greeter>("greeters", &none).is_empty());
        assert!(t.warnings("a").is_empty());
        assert!(t.warnings("b")[0].contains("no installed plugin declares it"));
        assert!(t.warnings("c")[0].contains("expects"));
    }

    struct OtherImpl;
    impl Other for OtherImpl {}

    struct Slow(Duration);
    impl Greeter for Slow {
        fn greet(&self) -> String {
            std::thread::sleep(self.0);
            "late".into()
        }
    }

    #[test]
    fn calls_with_their_own_budget_finish_or_are_abandoned() {
        let health = Arc::new(Health::default());
        let mut t = ServiceTable::default();
        let g = Some(Guard::new("p", health.clone()));
        t.provide::<dyn Greeter>("p", g.clone(), "quick", Version::new(1, 0, 0), Arc::new(Hello))
            .unwrap();
        let s = t.service::<dyn Greeter>("quick", &VersionReq::STAR, &ok).unwrap();
        assert_eq!(s.call_detached_within(Duration::from_secs(5), |g| g.greet()).unwrap(), "hello");
        let e = s
            .try_call_detached_within(Duration::from_secs(5), |_| Err::<(), _>(anyhow!("refused")))
            .unwrap_err();
        assert!(e.to_string().contains("refused"), "{e}");
        assert!(!health.is_quarantined("p"), "ordinary errors are not failures");

        // Past its budget the call is abandoned and the provider marked hung until it returns.
        let health = Arc::new(Health::default());
        let mut t = ServiceTable::default();
        t.provide::<dyn Greeter>(
            "slow",
            Some(Guard::new("slow", health.clone())),
            "slow",
            Version::new(1, 0, 0),
            Arc::new(Slow(Duration::from_millis(300))),
        )
        .unwrap();
        let s = t.service::<dyn Greeter>("slow", &VersionReq::STAR, &ok).unwrap();
        let e = s
            .call_detached_within(Duration::from_millis(20), |g| g.greet())
            .unwrap_err()
            .to_string();
        assert!(e.contains("did not finish within"), "{e}");
        assert!(health.availability("slow").is_err(), "hung while the abandoned call runs");
        // A longer budget lets the same call finish (once the abandoned one returned).
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while health.availability("slow").is_err() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(s.call_detached_within(Duration::from_secs(10), |g| g.greet()).unwrap(), "late");

        // Host services are not guarded: no budget.
        let mut t = ServiceTable::default();
        t.provide::<dyn Greeter>(HOST, None, "h", Version::new(1, 0, 0), Arc::new(Hello))
            .unwrap();
        let s = t.service::<dyn Greeter>("h", &VersionReq::STAR, &ok).unwrap();
        assert_eq!(s.call_detached_within(Duration::from_millis(1), |g| g.greet()).unwrap(), "hello");
    }
}
