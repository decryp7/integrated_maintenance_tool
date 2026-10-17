//! The contract tests, against a test-only provider plugin installed in a
//! real `PluginRegistry` and reached through the guarded service handle.
//!
//! No production plugin provides `CounterSource` yet: the Windows plugin
//! imports PerfMon logs as records but has no live counter access, and the
//! PDH binding lives in `imt-monitor`'s collector. `TestCounters` is a test
//! double with fixed, made-up values; it is never registered by the app.

use anyhow::{anyhow, Result};
use imt_api_perf::{contract_tests, CounterSample, CounterSource, SERVICE_ID, VERSION};
use imt_core::plugin::{ActivationCtx, Manifest, Service};
use imt_core::{Plugin, PluginRegistry};
use std::sync::{Arc, Mutex};

struct TestCounters;
impl CounterSource for TestCounters {
    fn counters(&self) -> Result<Vec<String>> {
        Ok(vec!["\\Test\\Constant".into(), "\\Test(a)\\Rate".into()])
    }
    fn sample(&self, paths: &[String]) -> Result<Vec<CounterSample>> {
        paths
            .iter()
            .map(|p| match p.as_str() {
                "\\Test\\Constant" => Ok(CounterSample {
                    path: p.clone(),
                    value: 1.0,
                    at_ms: 1,
                }),
                "\\Test(a)\\Rate" => Ok(CounterSample {
                    path: p.clone(),
                    value: 0.5,
                    at_ms: 1,
                }),
                other => Err(anyhow!("unknown counter \"{other}\"")),
            })
            .collect()
    }
}

struct Provider;
impl Plugin for Provider {
    fn id(&self) -> &'static str {
        "test.counters"
    }
    fn name(&self) -> &'static str {
        "Test counters"
    }
    fn version(&self) -> &'static str {
        "1.0.0"
    }
    fn manifest(&self) -> Manifest {
        Manifest::new(self.id(), self.version()).provides(SERVICE_ID, VERSION)
    }
    fn register(&self, r: &mut PluginRegistry) -> Result<()> {
        r.provide::<dyn CounterSource>(SERVICE_ID, VERSION, Arc::new(TestCounters))?;
        Ok(())
    }
}

#[derive(Default)]
struct Consumer(Mutex<Option<Service<dyn CounterSource>>>);
impl Plugin for Consumer {
    fn id(&self) -> &'static str {
        "test.consumer"
    }
    fn name(&self) -> &'static str {
        "Consumer"
    }
    fn version(&self) -> &'static str {
        "1.0.0"
    }
    fn manifest(&self) -> Manifest {
        Manifest::new(self.id(), self.version()).requires("test.counters", "^1")
    }
    fn register(&self, _: &mut PluginRegistry) -> Result<()> {
        Ok(())
    }
    fn activate(&self, ctx: &ActivationCtx) -> Result<()> {
        *self.0.lock().unwrap() = Some(ctx.service::<dyn CounterSource>(SERVICE_ID, "^1")?);
        Ok(())
    }
}

#[test]
fn a_provider_passes_the_contract_through_the_guarded_handle() {
    let consumer = Arc::new(Consumer::default());
    let mut r = PluginRegistry::empty();
    r.install_shared(consumer.clone()).unwrap();
    r.install(&Provider).unwrap();
    r.resolve();
    let service = consumer.0.lock().unwrap().clone().expect("activated with the service");
    assert_eq!(service.provider(), "test.counters");
    service.call(contract_tests).unwrap().unwrap();
    contract_tests(&TestCounters).unwrap();
}

struct Broken;
impl CounterSource for Broken {
    fn counters(&self) -> Result<Vec<String>> {
        Ok(vec!["bad".into(), "\\A\\B".into(), "\\A\\B".into()])
    }
    fn sample(&self, paths: &[String]) -> Result<Vec<CounterSample>> {
        if paths.first().is_some_and(|p| p == "\\") {
            panic!("sample exploded");
        }
        Ok(vec![CounterSample {
            path: "\\Other\\X".into(),
            value: f64::NAN,
            at_ms: 0,
        }])
    }
}

struct Unlisted;
impl CounterSource for Unlisted {
    fn counters(&self) -> Result<Vec<String>> {
        Err(anyhow!("no access"))
    }
    fn sample(&self, _: &[String]) -> Result<Vec<CounterSample>> {
        Ok(Vec::new())
    }
}

struct Silent;
impl CounterSource for Silent {
    fn counters(&self) -> Result<Vec<String>> {
        Ok(vec!["\\A\\B".into()])
    }
    fn sample(&self, paths: &[String]) -> Result<Vec<CounterSample>> {
        if paths.is_empty() {
            Err(anyhow!("nothing asked"))
        } else if paths[0] == "\\A\\B" {
            Ok(Vec::new())
        } else {
            Err(anyhow!("unknown"))
        }
    }
}

#[test]
fn violations_are_reported() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let errors = contract_tests(&Broken).unwrap_err().join("\n");
    std::panic::set_hook(prev);
    for part in [
        "\"bad\" is not a counter path",
        "listed twice",
        "sample([]) returned 1 values",
        "unrequested counter",
        "non-finite",
        "no sample time",
        "succeeded; unknown paths must be an error",
        "panicked",
    ] {
        assert!(errors.contains(part), "{part}: {errors}");
    }
    assert_eq!(contract_tests(&Unlisted).unwrap_err(), ["counters() failed: no access"]);
    let errors = contract_tests(&Silent).unwrap_err().join("\n");
    assert!(
        errors.contains("sample([]) failed") && errors.contains("second sample returned no values"),
        "{errors}"
    );
}
