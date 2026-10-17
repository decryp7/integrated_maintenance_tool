//! Windows Service integration (`windows-service` crate): install and
//! uninstall through the Service Control Manager (administrator rights
//! required, with clear errors otherwise), and the service entry point that
//! runs [`super::run_service`] until the SCM asks it to stop.
//!
//! The installed command line is `imt-agent.exe service-run --config <path>`.

use super::{explain_scm_error, run_service, DESCRIPTION, DISPLAY_NAME, SERVICE_NAME};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use windows_service::service::{
    ServiceAccess, ServiceControl, ServiceControlAccept, ServiceErrorControl, ServiceExitCode, ServiceInfo, ServiceStartType, ServiceState,
    ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
use windows_service::{define_windows_service, service_dispatcher};

fn os_code(e: &windows_service::Error) -> Option<i32> {
    match e {
        windows_service::Error::Winapi(io) => io.raw_os_error(),
        _ => None,
    }
}

fn explain(action: &str) -> impl Fn(windows_service::Error) -> String + '_ {
    move |e| explain_scm_error(os_code(&e), action, &e.to_string())
}

/// Registers the service (automatic start, LocalSystem).
pub fn install(name: &str, config: &Path) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("cannot find imt-agent.exe: {e}"))?;
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE)
        .map_err(explain("Installing the service"))?;
    let info = ServiceInfo {
        name: OsString::from(name),
        display_name: OsString::from(DISPLAY_NAME),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: exe,
        launch_arguments: vec![
            OsString::from("service-run"),
            OsString::from("--config"),
            config.as_os_str().to_owned(),
        ],
        dependencies: vec![],
        account_name: None,
        account_password: None,
    };
    let service = manager
        .create_service(&info, ServiceAccess::CHANGE_CONFIG)
        .map_err(explain("Installing the service"))?;
    service.set_description(DESCRIPTION).map_err(explain("Describing the service"))?;
    Ok(())
}

/// Stops (if running) and deletes the service.
pub fn uninstall(name: &str) -> Result<(), String> {
    let manager =
        ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT).map_err(explain("Uninstalling the service"))?;
    let service = manager
        .open_service(name, ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::DELETE)
        .map_err(explain("Uninstalling the service"))?;
    service.delete().map_err(explain("Uninstalling the service"))?;
    if service
        .query_status()
        .map(|s| s.current_state != ServiceState::Stopped)
        .unwrap_or(false)
    {
        let _ = service.stop();
    }
    Ok(())
}

static CONFIG: OnceLock<PathBuf> = OnceLock::new();

define_windows_service!(ffi_service_main, service_main);

/// Hands the process to the SCM (called for `service-run`); returns when the service stops.
pub fn dispatch(config: PathBuf) -> Result<(), String> {
    let _ = CONFIG.set(config);
    service_dispatcher::start(SERVICE_NAME, ffi_service_main)
        .map_err(|e| format!("{e} — `service-run` is started by the Windows Service Control Manager; use `imt-agent run` from a console"))
}

fn status(state: ServiceState, accept: ServiceControlAccept, code: u32) -> ServiceStatus {
    ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: state,
        controls_accepted: accept,
        exit_code: ServiceExitCode::Win32(code),
        checkpoint: 0,
        wait_hint: Duration::from_secs(90),
        process_id: None,
    }
}

fn service_main(_args: Vec<OsString>) {
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let handler = move |control| match control {
        ServiceControl::Stop | ServiceControl::Shutdown => {
            flag.store(true, Ordering::Relaxed);
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    };
    let Ok(handle) = service_control_handler::register(SERVICE_NAME, handler) else {
        return;
    };
    let accept = ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN;
    let _ = handle.set_service_status(status(ServiceState::Running, accept, 0));
    let code = match CONFIG.get() {
        Some(config) => match run_service(config, stop, false) {
            Ok(()) => 0,
            Err(e) => {
                // No console here: leave the reason next to the config.
                let _ = std::fs::write(config.with_extension("startup-error.txt"), e);
                1
            }
        },
        None => 1,
    };
    let _ = handle.set_service_status(status(ServiceState::Stopped, ServiceControlAccept::empty(), code));
}
