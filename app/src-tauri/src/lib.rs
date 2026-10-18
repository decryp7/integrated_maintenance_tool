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

mod activity;
mod agent_commands;
mod ai;
mod app_dirs;
mod app_state;
mod background;
mod clef;
mod config_diff;
mod demo;
mod dev_overrides;
mod diagnostics;
mod engine_errors;
mod insights;
mod instance;
mod lifecycle;
mod live;
mod logging;
mod menu;
mod notify;
mod panel_windows;
mod plugin_isolation;
mod pointer;
mod profile;
mod profiling;
mod query_cancel;
mod settings;
mod share;
mod skew_commands;
mod snapshots;
mod storage;
mod system;
mod tools;
mod tools_compat;
mod tray;
mod updater;
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
            query_cancel::run_query_stream,
            query_cancel::cancel_query,
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
            // Configuration diff (per window: its plugins' formats)
            config_diff::config_diff_formats,
            config_diff::diff_configs,
            config_diff::diff_snapshots,
            config_diff::export_config_diff,
            // Clock-skew correction (per window)
            skew_commands::set_source_offset,
            skew_commands::suggest_offsets,
            // Profiling: CPU/allocation profiles and heap snapshots (per window)
            profiling::profiling_open,
            profiling::profiling_sniff,
            profiling::profiling_frame_paths,
            profiling::profiling_list,
            profiling::profiling_close,
            profiling::profiling_spans,
            profiling::profiling_hotspots,
            profiling::profiling_tree,
            profiling::profiling_hot_path,
            profiling::profiling_butterfly,
            profiling::profiling_flame,
            profiling::profiling_diff_flame,
            profiling::profiling_function_diff,
            profiling::profiling_threads,
            profiling::profiling_findings,
            profiling::profiling_moment,
            profiling::profiling_heap_types,
            profiling::profiling_heap_diff,
            profiling::profiling_heap_growth,
            profiling::profiling_heap_dominators,
            profiling::profiling_heap_instances,
            profiling::profiling_heap_paths,
            // Profiles
            profile::get_profile,
            profile::validate_profile,
            profile::save_profile,
            profile::profile_history,
            profile::profile_version,
            profile::import_profile,
            profile::export_profile,
            profile::profile_templates,
            demo::demo_paths,
            // Error snapshots and diagnostics
            snapshots::save_error_snapshot,
            snapshots::list_error_snapshots,
            snapshots::read_error_snapshot,
            snapshots::delete_error_snapshot,
            snapshots::open_snapshots_folder,
            snapshots::export_diagnostics,
            // Settings ▸ Storage (see storage.rs)
            storage::storage_usage,
            storage::storage_clear,
            storage::storage_reveal,
            activity::report::problem_report_preview,
            activity::report::save_problem_report,
            activity::report::open_activity_log,
            diagnostics::diagnostics_settings,
            diagnostics::set_diagnostics_settings,
            // System and tools
            system::elevation,
            system::restart_elevated,
            system::read_file_head,
            tools::tools_list,
            tools::tool_invoke,
            // Deprecated wrappers over tool_invoke (see tools_compat)
            tools_compat::perfmon_presets,
            tools_compat::perfmon_generate,
            tools_compat::perfmon_install,
            tools_compat::perfmon_control,
            tools_compat::perfmon_outputs,
            tools_compat::monitor_start,
            tools_compat::monitor_stop,
            tools_compat::monitor_status,
            tools_compat::collect_default_plan,
            tools_compat::collect_diagnostics,
            // Scheduled collection (imt-agent)
            agent_commands::agent_schema,
            agent_commands::agent_config_load,
            agent_commands::agent_config_validate,
            agent_commands::agent_config_save,
            agent_commands::agent_runs,
            agent_commands::agent_run_manifest,
            agent_commands::agent_open_run,
            // Lifetime and sessions
            lifecycle::app_status,
            lifecycle::restart_app,
            lifecycle::plugin_reset,
            lifecycle::plugin_set_enabled,
            plugin_isolation::plugin_isolation_status,
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
            panel_windows::popout_panel,
            pointer::pointer_button_down,
            panel_windows::dock_panel,
            panel_windows::list_popouts,
            panel_windows::set_popout_on_top,
            tray::set_recent_projects,
            menu::set_app_menu,
            background::background_tasks,
            background::stop_background_task,
            // Local AI assistant (set-up, models, use cases, chat; see ai.rs)
            ai::commands::ai_status,
            ai::commands::ai_hardware,
            ai::commands::ai_recommend,
            ai::commands::ai_catalog,
            ai::commands::ai_setup_plan,
            ai::commands::ai_setup_run,
            ai::commands::ai_setup_cancel,
            ai::commands::ai_models,
            ai::commands::ai_model_activate,
            ai::commands::ai_model_delete,
            ai::commands::ai_model_add_local,
            ai::commands::ai_reset,
            ai::commands::ai_propose_regex,
            ai::commands::ai_propose_profile,
            ai::commands::ai_propose_pipeline,
            ai::commands::ai_propose_markers,
            ai::commands::ai_propose_extractors,
            ai::commands::ai_nl_to_query,
            ai::commands::ai_explain_event,
            ai::commands::ai_summarize,
            ai::commands::ai_root_causes,
            ai::commands::ai_cancel,
            ai::commands::ai_chat_sessions,
            ai::commands::ai_chat_new,
            ai::commands::ai_chat_get,
            ai::commands::ai_chat_delete,
            ai::commands::ai_chat_send,
            ai::commands::ai_chat_cancel,
            ai::commands::ai_chat_events_since,
            ai::commands::ai_chat_history,
            ai::commands::ai_chat_restore,
            ai::commands::ai_chat_export_markdown,
            ai::commands::ai_suggested_prompts,
            ai::commands::ai_approval_respond,
            ai::commands::ai_open_link,
            ai::commands::ai_set_brave_key,
            ai::commands::ai_clear_brave_key,
            ai::commands::ai_brave_key_set,
            ai::runtime_commands::ai_runtimes,
            ai::runtime_commands::ai_runtime_install,
            ai::runtime_commands::ai_runtime_cancel,
            ai::runtime_commands::ai_runtime_delete,
            ai::runtime_commands::ai_runtime_choose,
            // Explain and share (per window)
            share::set_redacted_view,
            share::preview_redaction,
            share::redaction_summary,
            share::dashboard_tile,
            share::export_report,
            share::save_visual,
            share::estimate_bundle,
            share::create_bundle,
            share::open_bundle,
            // Signed updates (see updater.rs)
            updater::updates_status,
            updater::updates_check,
            updater::updates_download,
            updater::updates_cancel,
            updater::updates_install_on_quit,
            updater::updates_restart,
            updater::updates_open_releases,
        ]
    };
}

/// Window size and position are remembered per window label; visibility is
/// not (start in the tray decides it).
fn window_state_plugin<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    use tauri_plugin_window_state::StateFlags;
    let builder = tauri_plugin_window_state::Builder::default()
        .with_state_flags(StateFlags::all() & !StateFlags::VISIBLE)
        .with_filter(|label| !panel_windows::is_popout(label));
    match app_dirs::window_state_file(dev_overrides::current().data_dir.as_deref()) {
        Some(file) => builder.with_filename(file.to_string_lossy()),
        None => builder,
    }
    .build()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Debug builds only: test-run overrides (browser arguments, data folder, paths to open); a bad value stops here.
    if let Err(e) = dev_overrides::init() {
        eprintln!("IMT cannot start: {e}");
        std::process::exit(2);
    }
    // `imt-app [--safe-mode] [--new-instance] [--new-window] <paths...>`
    let mut launch = instance::parse_args(std::env::args().skip(1), std::env::current_dir().ok().as_deref());
    launch.paths.extend(dev_overrides::current().open_paths.iter().cloned());
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
    // A crashed WebKit content process is logged (fatal) and its webview reloaded (macOS only in Tauri 2.12).
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    {
        builder = builder.on_web_content_process_terminate(window_manager::web_content_terminated);
    }
    builder
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(window_state_plugin())
        .plugin(updater::plugin())
        .setup(move |app| setup(app, &launch, safe_mode, single))
        .on_window_event(window_manager::on_window_event)
        // Every IPC command is logged (with the UI's operation id) before it runs: see `activity`.
        .invoke_handler(activity::logged(commands!()))
        .build(tauri::generate_context!())
        .expect("error while building IMT")
        .run(on_run_event);
}

fn setup(app: &mut tauri::App, launch: &instance::LaunchArgs, safe_mode: bool, single: bool) -> Result<(), Box<dyn std::error::Error>> {
    create_main_window(app)?;
    let diag = diagnostics::load(app_dirs::config(app.handle()).ok().as_deref());
    // `tracing` with CLEF to daily files; `log` records pass the activity logger (breadcrumbs, error
    // dedup) on their way to it, the webview's through the log plugin's command: see `logging`.
    logging::init(app.handle(), app_dirs::log(app.handle()).ok().as_deref(), &diag.log)?;
    activity::set_config(diag.activity);
    if let Ok(dir) = app_dirs::log(app.handle()) {
        diagnostics::prune_old_logs(&dir, diag.log.max_age_days, std::time::SystemTime::now());
    }
    lifecycle::install_panic_logging();
    let life = lifecycle::Lifecycle::begin_in(app_dirs::data(app.handle()).ok(), safe_mode, lifecycle::InstanceScope::of(single));
    let previous_crash = life.previous_crash;
    log::info!(Version = env!("CARGO_PKG_VERSION"), SafeMode = safe_mode, PreviousCrash = previous_crash, SingleInstance = single; "Application starting — IMT {}", env!("CARGO_PKG_VERSION"));

    // ---- managed state (app-wide) ----
    let shared = AppState::new(app_dirs::config(app.handle()).ok().map(|d| d.join("profiles")), safe_mode);
    let windows = windows::Windows::new(shared.factory());
    // The startup window opens the paths given on the command line.
    windows.get_or_create(windows::MAIN)?.queue(instance::existing(&launch.paths));
    app.manage(shared);
    app.manage(windows);
    app.manage(panel_windows::Popouts::default());
    app.manage(life);
    app.manage(background::BackgroundTasks::default());
    app.manage(window_prefs::SharedPrefs::default());
    app.manage(notify::Notifier::default());
    app.manage(tray::TrayState::default());
    app.manage(instance::SingleInstance(single));
    app.manage(query_cancel::QueryJobs::default());

    // ---- settings ----
    settings::init(app.handle());
    settings::register(
        app.handle(),
        std::sync::Arc::new(plugin_isolation::IsolationSection::new()),
        settings::SectionGroup::Plugins,
    )?;
    plugin_isolation::install(app.handle());
    window_prefs::register(app.handle())?;
    updater::init(app.handle());
    ai::init(app.handle(), safe_mode);
    snapshots::prune_on_startup(app.handle());

    // ---- background work, tray and the first window ----
    let handle = app.handle().clone();
    app.state::<background::BackgroundTasks>().subscribe(move |tasks| {
        let _ = handle.emit(background::TASKS_EVENT, tasks);
        tray::refresh(&handle);
    });
    tray::create(app.handle());
    menu::install(app.handle());
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

/// The startup window from `tauri.conf.json` (`create: false` there), built here so it gets the same
/// [`dev_overrides`] as every other window.
fn create_main_window(app: &tauri::App) -> tauri::Result<()> {
    let config = app.config().app.windows.iter().find(|w| w.label == windows::MAIN).cloned();
    let config = config.ok_or_else(|| tauri::Error::WindowNotFound)?;
    dev_overrides::apply(tauri::WebviewWindowBuilder::from_config(app, &config)?).build()?;
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
            log::info!("Application is shutting down");
            // Last: may hand over to the update installer (which ends the process on Windows).
            updater::on_exit(app);
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
        manage_in(app, profile_dir, false)
    }

    fn manage_in(app: tauri::App<MockRuntime>, profile_dir: Option<PathBuf>, safe_mode: bool) -> tauri::App<MockRuntime> {
        let shared = AppState::new(profile_dir, safe_mode);
        let windows = windows::Windows::new(shared.factory());
        windows.get_or_create(windows::MAIN).unwrap();
        app.manage(shared);
        app.manage(windows);
        app.manage(background::BackgroundTasks::default());
        app.manage(window_prefs::SharedPrefs::default());
        app.manage(notify::Notifier::default());
        app.manage(tray::TrayState::default());
        app.manage(query_cancel::QueryJobs::default());
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
            .invoke_handler(activity::logged(commands!()))
            .build(mock_context(noop_assets()))
            .unwrap();
        manage(app, profile_dir)
    }

    /// Like [`test_app_with_commands`], in safe mode (no plugins).
    pub fn test_app_safe_mode() -> tauri::App<MockRuntime> {
        let app = mock_builder()
            .on_window_event(window_manager::on_window_event)
            .invoke_handler(activity::logged(commands!()))
            .build(mock_context(noop_assets()))
            .unwrap();
        manage_in(app, None, true)
    }

    /// The origin of the app's own pages, which the IPC ACL treats as local: `tauri://localhost`, but
    /// `http://tauri.localhost` on Windows (and Android), where wry serves custom protocols over http.
    pub fn ipc_url() -> tauri::Url {
        let url = if cfg!(any(windows, target_os = "android")) {
            "http://tauri.localhost"
        } else {
            "tauri://localhost"
        };
        url.parse().unwrap()
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
