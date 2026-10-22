//! Whether Debrief runs with administrator rights, and on which platform.

use crate::ids::ELEVATION;
#[cfg(not(windows))]
use crate::Runner;
use anyhow::{bail, Result};
use debrief_plugin_api::tools::{OpSpec, Platform, Tool, ToolCtx, ToolInfo};
use serde_json::{json, Value};
use std::time::Duration;

pub(crate) struct ElevationTool {
    #[cfg(not(windows))]
    runner: Runner,
}

impl ElevationTool {
    pub fn new(runner: crate::Runner) -> ElevationTool {
        #[cfg(windows)]
        let _ = runner;
        ElevationTool {
            #[cfg(not(windows))]
            runner,
        }
    }

    /// Root on Unix-like systems (`id -u` prints 0).
    #[cfg(not(windows))]
    fn is_elevated(&self) -> bool {
        self.runner
            .run("id", &["-u".to_string()], Duration::from_secs(5))
            .map(|o| o.exit_code == Some(0) && String::from_utf8_lossy(&o.stdout).trim() == "0")
            .unwrap_or(false)
    }

    /// The process token is elevated (UAC).
    #[cfg(windows)]
    fn is_elevated(&self) -> bool {
        use windows::Win32::Foundation::{CloseHandle, HANDLE};
        use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
        use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
        // SAFETY: plain Win32 calls with owned, correctly sized buffers; the token handle is closed.
        unsafe {
            let mut token = HANDLE::default();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
                return false;
            }
            let mut elevation = TOKEN_ELEVATION::default();
            let mut len = 0u32;
            let ok = GetTokenInformation(
                token,
                TokenElevation,
                Some(&mut elevation as *mut _ as *mut core::ffi::c_void),
                std::mem::size_of::<TOKEN_ELEVATION>() as u32,
                &mut len,
            )
            .is_ok();
            let _ = CloseHandle(token);
            ok && elevation.TokenIsElevated != 0
        }
    }
}

impl Tool for ElevationTool {
    fn id(&self) -> &str {
        ELEVATION
    }

    fn describe(&self) -> ToolInfo {
        ToolInfo::new(ELEVATION, "Administrator rights")
            .describe("Whether Debrief runs elevated (as administrator or root), and the platform.")
            .op(OpSpec::new("info", "Elevation and platform").budget(Some(Duration::from_secs(10))))
    }

    fn invoke(&self, op: &str, _args: Value, _ctx: &ToolCtx) -> Result<Value> {
        match op {
            "info" => Ok(json!({
                "elevated": self.is_elevated(),
                "canRestartElevated": cfg!(windows),
                "platform": Platform::current(),
            })),
            other => bail!("unknown operation \"{other}\""),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(not(windows))]
    use crate::fake::fail;
    use crate::fake::{ok, FakeRunner};
    use debrief_plugin_api::tools::NoTasks;
    use std::sync::Arc;

    fn info(fake: FakeRunner) -> Value {
        let tool = ElevationTool::new(Arc::new(fake));
        assert!(tool.describe().check().is_ok());
        let ctx = ToolCtx::new(None, Arc::new(NoTasks));
        assert!(tool.invoke("nope", json!({}), &ctx).is_err());
        tool.invoke("info", json!({}), &ctx).unwrap()
    }

    #[test]
    fn reports_platform_and_rights() {
        let normal = info(FakeRunner::new(|_, _| ok("501\n")));
        assert_eq!(normal["canRestartElevated"], cfg!(windows));
        assert_eq!(normal["platform"], serde_json::to_value(Platform::current()).unwrap());
        #[cfg(not(windows))]
        {
            assert_eq!(normal["elevated"], false);
            assert_eq!(info(FakeRunner::new(|_, _| ok("0\n")))["elevated"], true);
            assert_eq!(info(FakeRunner::new(|_, _| fail(1, "0", "")))["elevated"], false);
            let missing = FakeRunner::new(|p, _| Err(debrief_collect::CommandError::NotFound(p.into())));
            assert_eq!(info(missing)["elevated"], false);
        }
    }
}
