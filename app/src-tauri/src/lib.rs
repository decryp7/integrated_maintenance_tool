//! Tauri shell: exposes the imt-core workspace to the Angular UI. The command
//! names and payloads are documented in app/src/app/core/api.types.ts.
//!
//! This file is the composition root only. Where things go:
//! - **Commands**: implement them in a module and add one line to [`commands!`].
//!   A command that works on a window's workspace takes
//!   [`windows::CurrentWindow`] (resolved from the calling window's label);
//!   app-wide state is a `State<'_, T>` of something managed in [`setup`].
//! - **Per-window state**: a field of [`windows::WindowState`] (created by
//!   [`AppState::new_window`], dropped when the window is destroyed).
//! - **App-wide state**: `app.manage(...)` in [`setup`], in the "managed
//!   state" block.
//! - **Background work** the user can see and stop (tray, status bar):
//!   register with [`background::BackgroundTasks`].
//! - **Settings sections**: `settings::register(...)` in [`setup`].
//! - **Events to one window**: [`windows::emit_to_window`]; never `emit` window-specific events to all.

mod app_state;
mod background;
mod clef;
mod diagnostics;
mod engine_errors;
mod insights;
mod instance;
mod lifecycle;
mod live;
mod notify;
mod profile;
mod settings;
mod snapshots;
mod system;
mod tools;
mod tray;
mod window_manager;
mod window_prefs;
mod windows;
mod workspace_commands;

pub(crate) use app_state::AppState;
pub(crate) use tauri::State;

use tauri::{Emitter, Manager};

pub(crate) type CmdResult<T> = Result<T, String>;

pub(crate) fn err(e: impl std::fmt::Display) -> String {
    format!("{e:#}")
}

/// Every command the UI can invoke. Add new commands here, one per line.
macro_rules! commands {
    () => {
        tauri::generate_handler![
            // Workspace (per window)
            workspace_commands::workspace_summary,
            workspace_commands::open_paths,
            workspace_commands::remove_source,
            workspace_commands::clear_workspace,
            workspace_commands::reload_workspace,
            workspace_commands::query_page,
            workspace_commands::query_timeline,
            workspace_commands::query_facet,
            workspace_commands::query_chart,
            workspace_commands::preview_extractor,
            workspace_commands::export_events,
            workspace_commands::source_context,
            workspace_commands::take_pending_open,
            workspace_commands::set_live,
            workspace_commands::import_progress,
            workspace_commands::cancel_import,
            workspace_commands::overview,
            workspace_commands::annotations,
            workspace_commands::plugin_events,
            workspace_commands::save_project,
            workspace_commands::open_project,
            live::tail_status,
            live::live_state,
            live::mute_alerts,
            live::unmute_alerts,
            live::open_paths_live,
            // Analysis (per window)
            insights::run_query,
            insights::check_query,
            insights::complete_query,
            insights::query_patterns,
            insights::query_exceptions,
            insights::query_anomalies,
            insights::compare_windows,
            insights::field_stats,
            insights::correlate,
            insights::query_traces,
            insights::query_requests,
            insights::query_moment,
            insights::preview_parse,
            insights::infer_regex,
            insights::explain_regex,
            // Profiles
            profile::get_profile,
            profile::validate_profile,
            profile::save_profile,
            profile::profile_history,
            profile::profile_version,
            profile::import_profile,
            profile::export_profile,
            // Error snapshots and diagnostics
            snapshots::save_error_snapshot,
            snapshots::list_error_snapshots,
            snapshots::read_error_snapshot,
            snapshots::delete_error_snapshot,
            snapshots::open_snapshots_folder,
            snapshots::export_diagnostics,
            diagnostics::diagnostics_settings,
            diagnostics::set_diagnostics_settings,
            // System and tools
            system::elevation,
            system::restart_elevated,
            system::read_file_head,
            tools::perfmon_presets,
            tools::perfmon_generate,
            tools::perfmon_install,
            tools::perfmon_control,
            tools::perfmon_outputs,
            tools::monitor_start,
            tools::monitor_stop,
            tools::monitor_status,
            tools::collect_default_plan,
            tools::collect_diagnostics,
            // Lifetime and sessions
            lifecycle::app_status,
            lifecycle::restart_app,
            lifecycle::plugin_reset,
            lifecycle::plugin_set_enabled,
            lifecycle::save_session,
            lifecycle::last_session,
            lifecycle::restore_windows,
            // Settings
            settings::settings_sections,
            settings::set_settings,
            settings::reset_settings,
            settings::export_settings,
            settings::import_settings,
            // Windows, tray and background work
            window_manager::new_window,
            window_manager::close_window,
            window_manager::list_windows,
            window_manager::focus_window,
            window_manager::set_window_title,
            window_manager::resolve_close,
            window_manager::resolve_open,
            tray::set_recent_projects,
            background::background_tasks,
            background::stop_background_task,
        ]
    };
}

/// Logs go to rolling files in the app's log directory (and stdout) as CLEF,
/// Serilog's compact JSON, including events forwarded from the UI. File size,
/// count and level come from the user's diagnostics settings.
fn log_plugin<R: tauri::Runtime>(limits: &diagnostics::LogLimits) -> tauri::plugin::TauriPlugin<R> {
    use tauri_plugin_log::{RotationStrategy, Target, TargetKind};
    tauri_plugin_log::Builder::new()
        .targets([
            Target::new(TargetKind::LogDir {
                file_name: Some(diagnostics::LOG_NAME.into()),
            }),
            Target::new(TargetKind::Stdout),
        ])
        // Everything reaches the dispatcher; `log::set_max_level` applies the
        // configured level, so it can change without a restart.
        .level(log::LevelFilter::Trace)
        .rotation_strategy(RotationStrategy::KeepSome(limits.keep_files as usize))
        .max_file_size(u128::from(limits.max_file_mb) * 1024 * 1024)
        .format(|out, message, record| {
            out.finish(format_args!(
                "{}",
                clef::format(chrono::Utc::now(), record, &message.to_string(), &clef::Enrichment::current())
            ))
        })
        .build()
}

/// Window size and position are remembered per window label; visibility is
/// not (start in the tray decides it).
fn window_state_plugin<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    use tauri_plugin_window_state::StateFlags;
    tauri_plugin_window_state::Builder::default()
        .with_state_flags(StateFlags::all() & !StateFlags::VISIBLE)
        .build()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // `imt-app [--safe-mode] [--new-instance] [--new-window] <paths...>`
    let launch = instance::parse_args(std::env::args().skip(1), std::env::current_dir().ok().as_deref());
    let safe_mode_env = std::env::var_os("IMT_SAFE_MODE").is_some();
    let single = instance::wants_single_instance(&launch, safe_mode_env);
    let safe_mode = launch.safe_mode || safe_mode_env;

    let mut builder = tauri::Builder::default();
    // Must be the first plugin: a second launch hands over and exits before anything else runs.
    #[cfg(any(target_os = "macos", windows, target_os = "linux"))]
    if single {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, argv, cwd| {
            instance::on_second_instance(app, argv, cwd)
        }));
    }
    builder
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(window_state_plugin())
        .setup(move |app| setup(app, &launch, safe_mode, single))
        .on_window_event(window_manager::on_window_event)
        .invoke_handler(commands!())
        .build(tauri::generate_context!())
        .expect("error while building IMT")
        .run(on_run_event);
}

fn setup(app: &mut tauri::App, launch: &instance::LaunchArgs, safe_mode: bool, single: bool) -> Result<(), Box<dyn std::error::Error>> {
    let diag = diagnostics::load(app.path().app_config_dir().ok().as_deref());
    app.handle().plugin(log_plugin(&diag.log))?;
    log::set_max_level(diag.log.level.filter());
    if let Ok(dir) = app.path().app_log_dir() {
        diagnostics::prune_old_logs(&dir, diag.log.max_age_days, std::time::SystemTime::now());
    }
    lifecycle::install_panic_logging();
    let life = lifecycle::Lifecycle::begin_in(app.path().app_data_dir().ok(), safe_mode, lifecycle::InstanceScope::of(single));
    let previous_crash = life.previous_crash;
    log::info!(Version = env!("CARGO_PKG_VERSION"), SafeMode = safe_mode, PreviousCrash = previous_crash, SingleInstance = single; "Application starting — IMT {}", env!("CARGO_PKG_VERSION"));

    // ---- managed state (app-wide) ----
    let shared = AppState::new(app.path().app_config_dir().ok().map(|d| d.join("profiles")), safe_mode);
    let windows = windows::Windows::new(shared.factory());
    // The startup window opens the paths given on the command line.
    windows.get_or_create(windows::MAIN)?.queue(instance::existing(&launch.paths));
    app.manage(shared);
    app.manage(windows);
    app.manage(life);
    app.manage(tools::Tools::default());
    app.manage(background::BackgroundTasks::default());
    app.manage(window_prefs::SharedPrefs::default());
    app.manage(notify::Notifier::default());
    app.manage(tray::TrayState::default());
    app.manage(instance::SingleInstance(single));

    // ---- settings ----
    settings::init(app.handle());
    window_prefs::register(app.handle())?;
    snapshots::prune_on_startup(app.handle());

    // ---- background work, tray and the first window ----
    let handle = app.handle().clone();
    app.state::<background::BackgroundTasks>().subscribe(move |tasks| {
        let _ = handle.emit(background::TASKS_EVENT, tasks);
        tray::refresh(&handle);
    });
    tray::create(app.handle());
    let in_tray = app.state::<window_prefs::SharedPrefs>().get().start_minimized && tray::available(app.handle());
    if let Some(main) = app.get_webview_window(windows::MAIN) {
        if !in_tray {
            main.show()?;
            let _ = main.set_focus();
        }
    }
    if previous_crash {
        notify::notify(
            app.handle(),
            notify::NotificationKind::CrashDetected,
            !in_tray,
            "IMT did not close properly",
            "Open IMT to restore your last session.",
        );
    }
    Ok(())
}

fn on_run_event(app: &tauri::AppHandle, event: tauri::RunEvent) {
    match event {
        tauri::RunEvent::ExitRequested { .. } => {
            if let Some(w) = app.try_state::<windows::Windows>() {
                w.set_exiting();
            }
        }
        tauri::RunEvent::Exit => {
            if let Some(w) = app.try_state::<windows::Windows>() {
                w.shutdown_plugins();
            }
            if let Some(life) = app.try_state::<lifecycle::Lifecycle>() {
                life.end();
            }
            log::info!("Application is shutting down")
        }
        // Files opened from Finder (double-click, "Open with").
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        tauri::RunEvent::Opened { urls } => {
            let paths: Vec<std::path::PathBuf> = urls.iter().filter_map(|u| u.to_file_path().ok()).collect();
            instance::open_files(app, instance::existing(&paths), false);
        }
        // The Dock icon was clicked while every window is hidden.
        #[cfg(target_os = "macos")]
        tauri::RunEvent::Reopen {
            has_visible_windows: false,
            ..
        } => instance::open_files(app, Vec::new(), false),
        _ => {}
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use std::path::PathBuf;
    use tauri::test::{mock_builder, mock_context, noop_assets, MockRuntime};

    pub fn samples() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../samples")
    }

    pub struct TempDir(pub PathBuf);
    impl TempDir {
        pub fn new(tag: &str) -> TempDir {
            let p = std::env::temp_dir().join(format!("imt-app-test-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            TempDir(p)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    pub fn block<F: std::future::Future>(f: F) -> F::Output {
        tauri::async_runtime::block_on(f)
    }

    fn manage(app: tauri::App<MockRuntime>, profile_dir: Option<PathBuf>) -> tauri::App<MockRuntime> {
        let shared = AppState::new(profile_dir, false);
        let windows = windows::Windows::new(shared.factory());
        windows.get_or_create(windows::MAIN).unwrap();
        app.manage(shared);
        app.manage(windows);
        app.manage(background::BackgroundTasks::default());
        app.manage(window_prefs::SharedPrefs::default());
        app.manage(notify::Notifier::default());
        app.manage(tray::TrayState::default());
        app
    }

    /// An app like the real one (managed state, window events), without windows.
    pub fn test_app(profile_dir: Option<PathBuf>) -> tauri::App<MockRuntime> {
        let app = mock_builder()
            .on_window_event(window_manager::on_window_event)
            .build(mock_context(noop_assets()))
            .unwrap();
        manage(app, profile_dir)
    }

    /// Like [`test_app`], with the real command handler (for IPC routing tests).
    pub fn test_app_with_commands(profile_dir: Option<PathBuf>) -> tauri::App<MockRuntime> {
        let app = mock_builder()
            .on_window_event(window_manager::on_window_event)
            .invoke_handler(commands!())
            .build(mock_context(noop_assets()))
            .unwrap();
        manage(app, profile_dir)
    }

    /// The main window's state (as a command would get it).
    pub fn main_window(app: &tauri::App<MockRuntime>) -> windows::CurrentWindow {
        windows::CurrentWindow::of(app.handle(), windows::MAIN).unwrap()
    }

    /// A (mock) window with this label, created when missing.
    pub fn window(app: &tauri::App<MockRuntime>, label: &str) -> tauri::WebviewWindow<MockRuntime> {
        app.get_webview_window(label).unwrap_or_else(|| {
            tauri::WebviewWindowBuilder::new(app, label, tauri::WebviewUrl::default())
                .build()
                .unwrap()
        })
    }
}
