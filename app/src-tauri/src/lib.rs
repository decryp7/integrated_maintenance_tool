//! Tauri shell: exposes the imt-core workspace to the Angular UI. The command
//! names and payloads are documented in app/src/app/core/api.types.ts.
//!
//! This file is the composition root only. Where things go:
//! - **Commands**: implement them (async) in a module and add one line to [`command_table!`] with their kind,
//!   windows and request limit.
//!   A command that works on a window's workspace takes
//!   [`windows::CurrentWindow`] (resolved from the calling window's label);
//!   app-wide state is a `State<'_, T>` of something managed in [`setup`].
//! - **Per-window state**: a field of [`windows::WindowState`] (created by
//!   [`AppState::new_window`], dropped when the window is destroyed).
//! - **App-wide state**: `app.manage(...)` in [`setup`], in the "managed
//!   state" block.
//! - **Background work** the user can see and stop (tray, status bar):
//!   register with [`background::BackgroundTasks`] when it runs until stopped; work that ends is a
//!   job ([`jobs::run`]), listed in the Jobs pane with progress, Cancel and Retry.
//! - **Settings sections**: `settings::register(...)` in [`setup`].
//! - **Events to one window**: [`windows::emit_to_window`]; never `emit` window-specific events to all.

mod activity;
mod agent_commands;
mod ai;
mod app_context;
mod app_dirs;
mod app_state;
mod background;
mod browser;
mod builtin_sections;
mod clef;
mod commands;
mod config_diff;
mod config_store;
mod demo;
mod dev_overrides;
mod diagnostics;
mod drag_ghost;
mod dumps;
mod engine;
mod engine_errors;
mod error;
mod file_commands;
#[cfg(test)]
mod generated_types;
mod insights;
mod instance;
mod jobs;
mod lifecycle;
mod live;
mod logging;
mod menu;
mod native_feel;
mod notes;
mod notification_centre;
mod notify;
mod opener;
mod panel_windows;
mod parsers;
mod paths;
mod performance;
mod plugin_config;
mod plugin_packages;
mod pointer;
mod profile;
mod profiling;
mod query_cancel;
mod recent;
mod services;
mod settings;
mod share;
mod skew_commands;
mod smart_search;
mod snapshots;
mod storage;
mod store_pane;
mod symbols;
mod sync;
mod system;
mod tools;
mod tray;
mod ui_dispatch;
mod updater;
mod video;
mod window_manager;
mod window_prefs;
mod window_scoped;
mod windows;
mod workspace_commands;

pub(crate) use app_state::AppState;
pub(crate) use tauri::State;

use tauri::{Emitter, Manager};

pub(crate) use error::{CmdError, CmdResult, ErrorCode};

pub(crate) fn err(e: impl std::fmt::Display) -> String {
    format!("{e:#}")
}

/// One mebibyte, for the table's request limits.
const MIB: usize = 1024 * 1024;

/// Declares the command table (see [`commands`]): `Kind [windows] (limit <bytes>)? => path,` per command. It
/// defines [`COMMANDS`] and the `handler!` macro that builds the invoke handler from the same list.
macro_rules! command_table {
    ($( $(#[$doc:meta])* $kind:ident [$($win:ident)|+] $(limit $limit:expr)? => $($seg:ident)::+ ),* $(,)?) => {
        /// Every command the UI can invoke, with its kind, windows and request limit.
        pub(crate) static COMMANDS: &[commands::Spec] = &[
            $( commands::Spec {
                path: stringify!($($seg)::+),
                kind: commands::Kind::$kind,
                windows: 0 $(| commands::$win)+,
                limit: commands::limit(&[$($limit)?]),
            }, )*
        ];
        macro_rules! handler {
            () => {
                tauri::generate_handler![ $( $($seg)::+ ),* ]
            };
        }
    };
}

// Every command the UI can invoke: one line each (ADR 0016). Kind: Query (read-only), Mutation, Quiet
// (polling and bookkeeping). Windows: MAIN (`main`, `w-*`), POPOUT (`p-*`), WINDOW =
// MAIN | POPOUT. Limit: the largest request in bytes (default `commands::DEFAULT_LIMIT`).
command_table! {
    Query [WINDOW] => workspace_commands::workspace_summary,
    Mutation [WINDOW] => workspace_commands::open_paths,
    Mutation [WINDOW] => workspace_commands::remove_source,
    Mutation [WINDOW] => workspace_commands::clear_workspace,
    Mutation [WINDOW] => workspace_commands::reload_workspace,
    Query [WINDOW] => workspace_commands::query_page,
    Query [WINDOW] => workspace_commands::query_timeline,
    Query [WINDOW] => workspace_commands::query_facet,
    Query [WINDOW] => workspace_commands::query_chart,
    Query [WINDOW] => workspace_commands::preview_extractor,
    Mutation [WINDOW] => workspace_commands::export_events,
    Query [WINDOW] => workspace_commands::source_context,
    Mutation [WINDOW] => file_commands::open_source_file,
    Quiet [MAIN] => workspace_commands::take_pending_open,
    Mutation [WINDOW] => workspace_commands::set_live,
    Quiet [WINDOW] => workspace_commands::import_progress,
    Query [WINDOW] => workspace_commands::overview,
    Query [WINDOW] => workspace_commands::annotations,
    Query [WINDOW] => workspace_commands::query_runs,
    Query [WINDOW] => workspace_commands::preview_spans,
    Query [WINDOW] => workspace_commands::plugin_events,
    Query [WINDOW] => workspace_commands::import_problems,
    Query [WINDOW] limit MIB => workspace_commands::evaluate_fix,
    Query [WINDOW] => workspace_commands::index_status,
    Query [WINDOW] => store_pane::store_overview,
    Query [WINDOW] => store_pane::store_segments,
    Query [WINDOW] => store_pane::store_fields,
    Query [WINDOW] => store_pane::store_index,
    Query [WINDOW] => store_pane::store_memory,
    Mutation [WINDOW] => store_pane::store_maintenance,
    Query [WINDOW] => store_pane::sql_schema,
    Query [WINDOW] => store_pane::sql_run,
    Query [WINDOW] => store_pane::sql_page,
    Query [WINDOW] => store_pane::sql_ids,
    Mutation [WINDOW] => store_pane::sql_export,
    Mutation [WINDOW] limit 16 * MIB => workspace_commands::save_project,
    Mutation [WINDOW] => workspace_commands::open_project,
    Query [WINDOW] => live::tail_status,
    Query [WINDOW] => live::live_state,
    Mutation [WINDOW] => live::mute_alerts,
    Mutation [WINDOW] => live::unmute_alerts,
    Mutation [WINDOW] => live::open_paths_live,
    Query [WINDOW] => insights::run_query,
    Query [WINDOW] => query_cancel::run_query_stream,
    Query [WINDOW] => smart_search::search_partial,
    Query [WINDOW] => insights::check_query,
    Query [WINDOW] => insights::complete_query,
    Query [WINDOW] => insights::query_patterns,
    Query [WINDOW] => insights::query_exceptions,
    Query [WINDOW] => insights::query_anomalies,
    Query [WINDOW] => insights::compare_windows,
    Query [WINDOW] => insights::field_stats,
    Query [WINDOW] => insights::correlate,
    Query [WINDOW] => insights::query_traces,
    Query [WINDOW] => insights::query_requests,
    Query [WINDOW] => insights::query_moment,
    Query [WINDOW] limit MIB => insights::preview_parse,
    Query [WINDOW] limit MIB => insights::infer_regex,
    Query [WINDOW] => insights::explain_regex,
    Query [WINDOW] => config_diff::config_diff_formats,
    Query [WINDOW] => config_diff::diff_configs,
    Query [WINDOW] limit 4 * MIB => config_diff::diff_snapshots,
    Mutation [WINDOW] => config_diff::export_config_diff,
    Mutation [WINDOW] => skew_commands::set_source_offset,
    Query [WINDOW] => skew_commands::suggest_offsets,
    Mutation [WINDOW] => profiling::profiling_open,
    Query [WINDOW] => profiling::profiling_sniff,
    Query [WINDOW] => profiling::profiling_frame_paths,
    Query [WINDOW] => profiling::profiling_list,
    Mutation [WINDOW] => profiling::profiling_close,
    Query [WINDOW] => profiling::profiling_spans,
    Query [WINDOW] => profiling::profiling_hotspots,
    Query [WINDOW] => profiling::profiling_tree,
    Query [WINDOW] => profiling::profiling_hot_path,
    Query [WINDOW] => profiling::profiling_butterfly,
    Query [WINDOW] => profiling::profiling_flame,
    Query [WINDOW] => profiling::profiling_diff_flame,
    Query [WINDOW] => profiling::profiling_function_diff,
    Query [WINDOW] => profiling::profiling_threads,
    Query [WINDOW] => profiling::profiling_findings,
    Query [WINDOW] => profiling::profiling_moment,
    Query [WINDOW] => profiling::profiling_heap_types,
    Query [WINDOW] => profiling::profiling_heap_diff,
    Query [WINDOW] => profiling::profiling_heap_growth,
    Query [WINDOW] => profiling::profiling_heap_dominators,
    Query [WINDOW] => profiling::profiling_heap_instances,
    Query [WINDOW] => profiling::profiling_heap_paths,
    Query [WINDOW] limit 4 * 1024 => profiling::profiling_heap_findings,
    Query [WINDOW] => video::runtime_commands::video_runtime,
    Mutation [WINDOW] => video::runtime_commands::video_runtime_install,
    Mutation [WINDOW] => video::runtime_commands::video_runtime_rollback,
    Mutation [WINDOW] => video::runtime_commands::video_runtime_delete,
    Mutation [WINDOW] => video::runtime_commands::video_runtime_set_custom,
    Mutation [WINDOW] => video::runtime_commands::video_runtime_clear_custom,
    Query [WINDOW] => video::commands::video_sniff,
    Query [WINDOW] => video::commands::video_list,
    Mutation [WINDOW] => video::commands::video_open,
    Query [WINDOW] => video::commands::video_frame_image,
    Mutation [WINDOW] => video::commands::video_frame_to_image,
    Mutation [WINDOW] => video::commands::video_set_alignment,
    Mutation [WINDOW] => video::commands::video_set_note,
    Mutation [WINDOW] => video::commands::video_remove_frame,
    Mutation [WINDOW] => video::commands::video_add_frame,
    Mutation [WINDOW] => video::commands::video_include,
    Mutation [WINDOW] => video::commands::video_remove,
    Mutation [WINDOW] => video::commands::video_describe,
    Mutation [WINDOW] => video::commands::video_read_clock,
    Query [WINDOW] => video::commands::video_explain,
    Query [WINDOW] => dumps::dumps_sniff,
    Mutation [WINDOW] => dumps::dumps_open,
    Query [WINDOW] => dumps::dumps_list,
    Mutation [WINDOW] => dumps::dumps_close,
    Query [WINDOW] => dumps::dumps_overview,
    Query [WINDOW] => dumps::dumps_threads,
    Query [WINDOW] => dumps::dumps_stack,
    Query [WINDOW] => dumps::dumps_modules,
    Query [WINDOW] => dumps::dumps_findings,
    Query [WINDOW] => dumps::dumps_umdh_diff,
    Query [WINDOW] => dumps::dumps_umdh_stack,
    Query [WINDOW] => dumps::dumps_umdh_tree,
    Query [WINDOW] => dumps::dumps_markers,
    Query [WINDOW] => dumps::dumps_explain,
    Query [WINDOW] => profile::get_profile,
    Query [WINDOW] limit MIB => profile::validate_profile,
    Mutation [WINDOW] limit MIB => profile::save_profile,
    Query [WINDOW] => profile::profile_history,
    Query [WINDOW] => profile::profile_version,
    Mutation [WINDOW] => profile::import_profile,
    Mutation [WINDOW] limit MIB => profile::export_profile,
    Query [WINDOW] => parsers::commands::parsers_list,
    Query [WINDOW] => parsers::commands::parser_get,
    Query [WINDOW] limit MIB => parsers::commands::parser_validate,
    Mutation [WINDOW] limit MIB => parsers::commands::parser_save,
    Mutation [WINDOW] => parsers::commands::parser_duplicate,
    Mutation [WINDOW] => parsers::commands::parser_remove,
    Mutation [WINDOW] => parsers::commands::parser_set_enabled,
    Query [WINDOW] => parsers::commands::parser_import_check,
    Mutation [WINDOW] => parsers::commands::parser_export,
    Query [WINDOW] limit MIB => parsers::commands::parser_draft,
    Mutation [WINDOW] => parsers::commands::parsers_reread,
    Mutation [WINDOW] => parsers::commands::parser_assign,
    Query [WINDOW] => demo::demo_paths,
    Mutation [WINDOW] limit 8 * MIB => snapshots::save_error_snapshot,
    Query [WINDOW] => snapshots::list_error_snapshots,
    Query [WINDOW] => snapshots::read_error_snapshot,
    Mutation [WINDOW] => snapshots::delete_error_snapshot,
    Mutation [WINDOW] => snapshots::open_snapshots_folder,
    Mutation [WINDOW] => snapshots::export_diagnostics,
    Query [WINDOW] => storage::storage_usage,
    Mutation [MAIN] => storage::storage_clear,
    Mutation [WINDOW] => storage::storage_reveal,
    Query [WINDOW] limit MIB => activity::report::problem_report_preview,
    Mutation [WINDOW] limit MIB => activity::report::save_problem_report,
    Mutation [WINDOW] => activity::report::open_activity_log,
    Query [WINDOW] => activity::report::open::diagnostics_report,
    Query [WINDOW] => activity::report::open::diagnostics_report_image,
    Query [WINDOW] => diagnostics::diagnostics_settings,
    Mutation [WINDOW] => diagnostics::set_diagnostics_settings,
    Query [WINDOW] => system::elevation,
    Mutation [MAIN] => system::restart_elevated,
    Mutation [WINDOW] => paths::pick_paths,
    Query [WINDOW] => system::read_file_head,
    Query [WINDOW] => tools::tools_list,
    Mutation [WINDOW] limit MIB => tools::tool_invoke,
    Quiet [WINDOW] limit 64 * 1024 => tools::tool_poll,
    Query [WINDOW] limit 4 * 1024 => tools::tool_task_job,
    Query [WINDOW] => agent_commands::agent_schema,
    Query [WINDOW] => agent_commands::agent_config_load,
    Query [WINDOW] limit MIB => agent_commands::agent_config_validate,
    Mutation [WINDOW] limit MIB => agent_commands::agent_config_save,
    Query [WINDOW] => agent_commands::agent_runs,
    Query [WINDOW] => agent_commands::agent_run_manifest,
    Mutation [WINDOW] => agent_commands::agent_open_run,
    Query [WINDOW] => lifecycle::app_status,
    Mutation [MAIN] => lifecycle::restart_app,
    Mutation [MAIN] => plugin_config::plugin_reset,
    Mutation [MAIN] => plugin_config::plugin_set_enabled,
    Query [MAIN] => plugin_packages::plugin_packages_list,
    Query [MAIN] => plugin_packages::plugin_package_inspect,
    Mutation [MAIN] => plugin_packages::plugin_package_install,
    Mutation [MAIN] => plugin_packages::plugin_package_remove,
    Mutation [MAIN] => plugin_packages::plugin_package_set_enabled,
    Mutation [MAIN] => plugin_packages::plugin_package_set_permissions,
    Quiet [MAIN] limit 16 * MIB => lifecycle::save_session,
    Query [MAIN] => lifecycle::last_session,
    Mutation [MAIN] => lifecycle::restore_windows,
    Query [WINDOW] => settings::settings_sections,
    Mutation [WINDOW] limit MIB => settings::set_settings,
    Mutation [MAIN] => settings::reset_settings,
    Mutation [WINDOW] => settings::export_settings,
    Mutation [MAIN] => settings::import_settings,
    Mutation [WINDOW] => window_manager::new_window,
    Mutation [WINDOW] => window_manager::close_window,
    Mutation [WINDOW] => window_manager::quit_app,
    Query [WINDOW] => window_manager::list_windows,
    Mutation [WINDOW] => window_manager::focus_window,
    Quiet [MAIN] => window_manager::set_window_title,
    Mutation [MAIN] => window_manager::resolve_close,
    Mutation [MAIN] => window_manager::resolve_open,
    Mutation [MAIN] => panel_windows::popout_panel,
    Quiet [WINDOW] => pointer::pointer_button_down,
    Quiet [MAIN] limit 3 * MIB => drag_ghost::drag_ghost_start,
    Quiet [MAIN] => drag_ghost::drag_ghost_show,
    Quiet [MAIN] => drag_ghost::drag_ghost_end,
    Mutation [WINDOW] => panel_windows::dock_panel,
    Query [WINDOW] => panel_windows::list_popouts,
    Mutation [WINDOW] => panel_windows::set_popout_on_top,
    Query [WINDOW] => recent::recent_list,
    Mutation [WINDOW] => recent::recent_remove,
    Mutation [WINDOW] => recent::recent_clear,
    Quiet [MAIN] limit MIB => menu::set_app_menu,
    Query [WINDOW] => background::background_tasks,
    Mutation [WINDOW] => background::stop_background_task,
    Query [WINDOW] => jobs::jobs_list,
    Mutation [WINDOW] => jobs::jobs_cancel,
    Mutation [WINDOW] => jobs::jobs_retry,
    Mutation [WINDOW] => jobs::jobs_move,
    Mutation [WINDOW] => jobs::jobs_clear_finished,
    Mutation [WINDOW] => jobs::jobs_mark_seen,
    Mutation [WINDOW] => jobs::jobs_result,
    Mutation [WINDOW] => jobs::jobs_reveal,
    Quiet [WINDOW] => jobs::jobs_record,
    Mutation [WINDOW] limit 4 * MIB => jobs::jobs_restore,
    Query [WINDOW] => jobs::jobs_wait,
    Query [WINDOW] => jobs::jobs_unwait,
    Query [WINDOW] => notification_centre::notifications_list,
    Mutation [WINDOW] => notification_centre::notifications_mark_read,
    Mutation [WINDOW] => notification_centre::notifications_mark_all_read,
    Mutation [WINDOW] => notification_centre::notifications_dismiss,
    Quiet [WINDOW] => notification_centre::notifications_record,
    Mutation [WINDOW] limit 4 * MIB => notification_centre::notifications_restore,
    Query [WINDOW] => performance::commands::performance_suggest,
    Query [WINDOW] => performance::commands::performance_preset,
    Query [WINDOW] => performance::commands::performance_memory,
    Mutation [WINDOW] => performance::commands::performance_set_plugin_memory,
    Mutation [WINDOW] => performance::commands::performance_retry_store_worker,
    Query [WINDOW] => browser::research::ai_research_draft,
    Mutation [WINDOW] => browser::research::ai_research,
    Query [WINDOW] => browser::driving::browser_driving,
    Mutation [WINDOW] => browser::commands::browser_open_and_read,
    Query [WINDOW] => ai::commands::ai_status,
    Query [WINDOW] => ai::commands::ai_hardware,
    Query [WINDOW] => ai::commands::ai_recommend,
    Query [WINDOW] => ai::commands::ai_catalog,
    Query [WINDOW] => ai::commands::ai_setup_plan,
    Mutation [WINDOW] => ai::commands::ai_setup_run,
    Query [WINDOW] => ai::commands::ai_models,
    Mutation [WINDOW] => ai::commands::ai_model_activate,
    Mutation [WINDOW] => ai::commands::ai_model_delete,
    Mutation [WINDOW] => ai::commands::ai_model_add_local,
    Mutation [WINDOW] => ai::commands::ai_reset,
    Mutation [WINDOW] => ai::commands::ai_start,
    Mutation [WINDOW] => ai::commands::ai_cancel,
    Query [WINDOW] => ai::folder_setup::folder_survey,
    Query [WINDOW] => ai::folder_setup::folder_setup_compose,
    Query [WINDOW] => ai::commands::ai_chat_sessions,
    Mutation [WINDOW] => ai::commands::ai_chat_new,
    Query [WINDOW] => ai::commands::ai_chat_get,
    Mutation [WINDOW] => ai::commands::ai_chat_delete,
    Mutation [WINDOW] => ai::commands::ai_chat_send,
    Mutation [WINDOW] => ai::commands::ai_chat_cancel,
    Quiet [WINDOW] => ai::commands::ai_chat_events_since,
    Query [WINDOW] => ai::commands::ai_chat_history,
    Mutation [WINDOW] limit 8 * MIB => ai::commands::ai_chat_restore,
    Mutation [WINDOW] => ai::commands::ai_chat_export_markdown,
    Query [WINDOW] => ai::commands::ai_suggested_prompts,
    Mutation [WINDOW] => ai::commands::ai_open_link,
    Query [WINDOW] => ai::knowledge::commands::ai_knowledge_list,
    Query [WINDOW] => ai::knowledge::commands::ai_knowledge_get,
    Mutation [WINDOW] limit MIB => ai::knowledge::commands::ai_knowledge_create,
    Mutation [WINDOW] => ai::knowledge::commands::ai_knowledge_import,
    Mutation [WINDOW] limit MIB => ai::knowledge::commands::ai_knowledge_update,
    Mutation [WINDOW] => ai::knowledge::commands::ai_knowledge_remove,
    Mutation [WINDOW] => ai::knowledge::commands::ai_knowledge_export,
    Query [WINDOW] => ai::knowledge::commands::ai_knowledge_project_record,
    Mutation [WINDOW] limit 16 * MIB => ai::knowledge::commands::ai_knowledge_project_restore,
    Query [WINDOW] => notes::commands::notes_list,
    Mutation [WINDOW] limit MIB => notes::commands::notes_add,
    Mutation [WINDOW] limit MIB => notes::commands::notes_update,
    Mutation [WINDOW] => notes::commands::notes_remove,
    Query [WINDOW] => notes::commands::notes_record,
    Mutation [WINDOW] limit 16 * MIB => notes::commands::notes_restore,
    Mutation [WINDOW] => notes::commands::notes_image_open,
    Query [WINDOW] => notes::commands::notes_image_get,
    Mutation [WINDOW] => ai::vision::ai_image_open,
    Mutation [WINDOW] limit 48 * MIB => ai::vision::ai_image_paste,
    Mutation [WINDOW] => ai::vision::ai_image_close,
    Mutation [WINDOW] => ai::vision::ai_model_install_projector,
    Mutation [WINDOW] => ai::vision::ai_model_add_projector,
    Query [WINDOW] => browser::commands::browser_state,
    Mutation [WINDOW] => browser::commands::browser_open,
    Query [WINDOW] => browser::commands::browser_event_query,
    Mutation [WINDOW] => browser::commands::browser_nav,
    Mutation [WINDOW] => browser::commands::browser_close,
    Quiet [WINDOW] => browser::commands::browser_place,
    Mutation [WINDOW] => browser::commands::browser_open_external,
    Mutation [WINDOW] => browser::commands::browser_capture,
    Query [WINDOW] => browser::commands::browser_sources,
    Mutation [WINDOW] => browser::commands::browser_discard_sources,
    Query [WINDOW] => browser::commands::browser_chat_status,
    Mutation [WINDOW] => browser::commands::browser_set_read_along,
    Mutation [WINDOW] => browser::commands::browser_set_agent,
    Mutation [WINDOW] => browser::commands::browser_agent_stop,
    Mutation [WINDOW] => browser::commands::browser_agent_continue,
    Mutation [WINDOW] => browser::commands::browser_agent_respond,
    Mutation [WINDOW] => browser::commands::browser_open_suggested,
    Query [WINDOW] => ai::runtime_commands::ai_runtimes,
    Mutation [WINDOW] => ai::runtime_commands::ai_runtime_install,
    Mutation [WINDOW] => ai::runtime_commands::ai_runtime_add_archives,
    Query [WINDOW] => ai::manual::ai_manual_download,
    Mutation [WINDOW] => ai::manual::ai_manual_download_open,
    Mutation [WINDOW] => ai::runtime_commands::ai_runtime_delete,
    Mutation [WINDOW] => ai::runtime_commands::ai_runtime_choose,
    Query [WINDOW] => ai::update_commands::ai_updates_status,
    Mutation [WINDOW] => ai::update_commands::ai_updates_check,
    Mutation [WINDOW] => ai::update_commands::ai_updates_apply,
    Mutation [WINDOW] => ai::update_commands::ai_updates_skip,
    Mutation [WINDOW] => ai::update_commands::ai_updates_later,
    Mutation [WINDOW] => share::set_redacted_view,
    Query [WINDOW] => share::preview_redaction,
    Query [WINDOW] => share::redaction_summary,
    Query [WINDOW] => share::dashboard_tile,
    Mutation [WINDOW] limit 32 * MIB => share::export_report,
    Mutation [WINDOW] limit 32 * MIB => share::save_visual,
    Query [WINDOW] => share::estimate_bundle,
    Mutation [WINDOW] limit 32 * MIB => share::create_bundle,
    Mutation [WINDOW] => share::open_bundle,
    Query [WINDOW] => updater::updates_status,
    Query [WINDOW] => updater::updates_check,
    Mutation [WINDOW] => updater::updates_download,
    Mutation [WINDOW] => updater::updates_discard,
    Mutation [MAIN] => updater::updates_install_on_quit,
    Mutation [MAIN] => updater::updates_restart,
    Mutation [WINDOW] => updater::updates_open_releases,
}

/// Window size and position are remembered per window label; visibility is
/// not (start in the tray decides it).
fn window_state_plugin<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    use tauri_plugin_window_state::StateFlags;
    let builder = tauri_plugin_window_state::Builder::default()
        .with_state_flags(StateFlags::all() & !StateFlags::VISIBLE)
        .with_filter(|label| !window_manager::is_auxiliary(label));
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
    // `imt-app --export-diagnostics [<path.zip>] [--force]`: the problem report without the UI, then exit.
    if let Some(code) = activity::report::cli::run_if_requested() {
        std::process::exit(code);
    }
    // `imt-app [--safe-mode] [--new-instance] [--new-window] <paths...>`
    let mut launch = instance::parse_args(std::env::args().skip(1), std::env::current_dir().ok().as_deref());
    launch.paths.extend(dev_overrides::current().open_paths.iter().cloned());
    let safe_mode_env = std::env::var_os("IMT_SAFE_MODE").is_some();
    let single = instance::wants_single_instance(&launch, safe_mode_env);
    let safe_mode = launch.safe_mode || safe_mode_env;

    let mut context = tauri::generate_context!();
    // Debug test runs with their own data folder also get their own identifier, which keys the single-instance lock.
    let identifier = dev_overrides::instance_identifier(&context.config().identifier, dev_overrides::current().data_dir.as_deref());
    context.config_mut().identifier = identifier;

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
        // The player's videos (ADR 0033 §7): only the calling window's project videos, in bounded ranges.
        .register_asynchronous_uri_scheme_protocol(video::media::SCHEME, video::media::handler)
        .setup(move |app| setup(app, &launch, safe_mode, single))
        .on_window_event(window_manager::on_window_event)
        // Every IPC command is logged (with the UI's operation id) before it runs: see `activity`. A web page
        // in the internal browser gets no command at all: see `browser::guard`.
        .invoke_handler(browser::guard(commands::guard(activity::logged(handler!()))))
        .build(context)
        .expect("error while building IMT")
        .run(on_run_event);
}

fn setup(app: &mut tauri::App, launch: &instance::LaunchArgs, safe_mode: bool, single: bool) -> Result<(), Box<dyn std::error::Error>> {
    create_main_window(app)?;
    // IMT's folders and run flags, resolved once (see `app_context`).
    let ctx = app_context::AppContext::of(app.handle(), safe_mode, single);
    let diag = diagnostics::load(ctx.config.as_deref());
    // `tracing` with CLEF to daily files; `log` records pass the activity logger (breadcrumbs, error
    // dedup) on their way to it, the webview's through the log plugin's command: see `logging`.
    logging::init(app.handle(), ctx.log.as_deref(), &diag.log)?;
    activity::set_config(diag.activity);
    // Child processes' stderr and start/exit lines (for problem reports): `<logs>/children/<name>.log`.
    if let Some(dir) = &ctx.log {
        imt_ipc::child_log::set_dir(activity::report::children_dir(dir));
        diagnostics::prune_old_logs(dir, diag.log.max_age_days, std::time::SystemTime::now());
    }
    lifecycle::install_panic_logging();
    let life = lifecycle::Lifecycle::begin_in(ctx.data.clone(), ctx.safe_mode, lifecycle::InstanceScope::of(ctx.single_instance));
    let previous_crash = life.previous_crash;
    log::info!(Version = env!("CARGO_PKG_VERSION"), SafeMode = safe_mode, PreviousCrash = previous_crash, SingleInstance = single; "Application starting — IMT {}", env!("CARGO_PKG_VERSION"));

    // ---- managed state (app-wide) ----
    // The store worker's libduckdb ships in the resource folder `lib/` (tauri.sidecars.conf.json).
    let lib_dir = tauri::Manager::path(app).resource_dir().ok().map(|d| d.join("lib"));
    let shared = AppState::new(
        ctx.config.as_ref().map(|d| d.join("profiles")),
        ctx.data.clone(),
        lib_dir,
        ctx.data.as_ref().map(|d| d.join("plugins")),
        ctx.safe_mode,
    )?;
    if let Some(problem) = shared.plugins.problem() {
        log::error!("Plugins are unavailable: {problem}");
    }
    // Plugin processes start in the background, so the first import does not wait for one.
    shared.plugins.warm_up();
    // Scratch stores no window or session will use again (left by a crash) are removed in the background.
    {
        let (root, keep) = (shared.store_root.clone(), life.session_stores());
        std::thread::spawn(move || engine::sweep_scratch(&root, &keep));
    }
    app_context::install(app.handle(), ctx);
    let windows = windows::Windows::new(shared.factory());
    // The startup window opens the paths given on the command line.
    windows.get_or_create(windows::MAIN)?.queue(instance::existing(&launch.paths));
    app.manage(shared);
    app.manage(windows);
    app.manage(panel_windows::Popouts::default());
    app.manage(drag_ghost::DragGhosts::default());
    app.manage(life);
    app.manage(background::BackgroundTasks::default());
    app.manage(window_prefs::SharedPrefs::default());
    app.manage(notify::Notifier::default());
    app.manage(tray::TrayState::default());
    jobs::init(app.handle());
    notification_centre::init(app.handle());
    performance::attach_notifications(app.handle());

    // ---- settings ----
    settings::init(app.handle());
    symbols::init(app.handle());
    plugin_packages::init(app.handle());
    window_prefs::register(app.handle())?;
    updater::init(app.handle());
    ai::init(app.handle(), safe_mode);
    video::init(app.handle());
    browser::init(app.handle());
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
    if let Some(main) = app.get_window(windows::MAIN) {
        if !in_tray {
            main.show()?;
            let _ = main.set_focus();
        }
    }
    session_notices(app.handle(), previous_crash, safe_mode);
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

/// A recovered crash and safe mode are app messages in the notification centre (the lifecycle banner
/// offers the restore itself).
fn session_notices<R: tauri::Runtime>(app: &tauri::AppHandle<R>, previous_crash: bool, safe_mode: bool) {
    use notification_centre::centre::{NewNotice, NoticeScope, NoticeSeverity, NoticeSource};
    if previous_crash {
        let n = NewNotice::new(
            NoticeSource::Session,
            NoticeSeverity::Warning,
            "IMT did not close properly",
            NoticeScope::App,
        )
        .body("Your last session can be restored; jobs that were running are listed as interrupted, with Retry.")
        .dedupe(format!("session:crash:{}", activity::session_id()));
        notification_centre::post(app, None, n, None);
    }
    if safe_mode {
        let n = NewNotice::new(
            NoticeSource::Session,
            NoticeSeverity::Info,
            "IMT runs in safe mode",
            NoticeScope::App,
        )
        .body("Plugins and the AI are off for this run. Restart IMT normally to turn them on again.")
        .dedupe(format!("session:safe:{}", activity::session_id()));
        notification_centre::post(app, None, n, None);
    }
}

/// The startup window from `tauri.conf.json` (`create: false` there), built here so it gets the same
/// [`dev_overrides`] and [`native_feel`] as every other window.
fn create_main_window(app: &tauri::App) -> tauri::Result<()> {
    let config = app.config().app.windows.iter().find(|w| w.label == windows::MAIN).cloned();
    let config = config.ok_or_else(|| tauri::Error::WindowNotFound)?;
    native_feel::build(tauri::WebviewWindowBuilder::from_config(app, &config)?)?;
    Ok(())
}

fn on_run_event(app: &tauri::AppHandle, event: tauri::RunEvent) {
    match event {
        tauri::RunEvent::ExitRequested { code, .. } => {
            log::info!(Code = code.unwrap_or_default(); "Exit requested");
            if let Some(w) = app.try_state::<windows::Windows>() {
                w.set_exiting();
            }
            lifecycle::arm_exit_watchdog(app, code.unwrap_or_default());
        }
        tauri::RunEvent::Exit => {
            log::info!("Exit: tearing down");
            lifecycle::exit_teardown(app);
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

    /// `.now()`: runs a command's future to its end (commands are async; tests call them directly).
    pub trait Now: std::future::Future + Sized {
        fn now(self) -> Self::Output {
            block(self)
        }
    }

    impl<F: std::future::Future> Now for F {}

    /// `.json()`: a typed reply as the JSON the UI gets (tests index it like the UI reads it).
    pub trait Json: serde::Serialize {
        fn json(&self) -> serde_json::Value {
            serde_json::to_value(self).unwrap()
        }
    }

    impl<T: serde::Serialize> Json for T {}

    /// Imports `paths` into the window's store and waits for it (tests set up a window without the UI).
    pub fn import(win: &windows::WindowState, paths: &[PathBuf]) -> imt_core::ports::ImportReport {
        let store = win.engine.store().unwrap();
        let report = store.open(paths).unwrap().wait().unwrap();
        win.set_loaded(true);
        report
    }

    /// The window's store summary (what the store has, without the host's parts).
    pub fn store_summary(win: &windows::WindowState) -> imt_core::ports::SummaryReply {
        win.engine.store().unwrap().summary().unwrap()
    }

    /// Waits for the test app's job `id` to end; how it ended.
    pub fn job_end(app: &tauri::App<MockRuntime>, id: u64) -> jobs::service::JobOutcome {
        use tauri::Manager;
        app.state::<jobs::Jobs>().wait_end(jobs::JobId(id))
    }

    /// Waits for job `id` to succeed; the line its message shows.
    pub fn job_done(app: &tauri::App<MockRuntime>, id: u64) -> String {
        let end = job_end(app, id);
        assert_eq!(end.state, jobs::JobState::Done, "{end:?}");
        end.message.unwrap_or_default()
    }

    /// The clock of the test app's jobs and notification centre.
    pub struct TestClock(pub std::sync::Arc<jobs::ManualClock>);

    static NEXT_TEST_APP: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

    fn manage(app: tauri::App<MockRuntime>, profile_dir: Option<PathBuf>) -> tauri::App<MockRuntime> {
        manage_in(app, profile_dir, false)
    }

    fn manage_in(app: tauri::App<MockRuntime>, profile_dir: Option<PathBuf>, safe_mode: bool) -> tauri::App<MockRuntime> {
        let plugins = std::sync::Arc::new(if safe_mode {
            imt_plugin_host::Builtins::none()
        } else {
            imt_plugin_host::Builtins::load(&imt_plugin_host::builtins::default_binary())
        });
        // The store in process, scratch stores under the system temp folder (one folder per test app).
        let root = std::env::temp_dir().join(format!(
            "imt-test-stores-{}-{}",
            std::process::id(),
            NEXT_TEST_APP.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ));
        let stores = engine::local::with_plugins(plugins.clone(), safe_mode);
        let shared = AppState::with(profile_dir, safe_mode, plugins, stores, root);
        let windows = windows::Windows::new(shared.factory());
        windows.get_or_create(windows::MAIN).unwrap();
        app.manage(shared);
        app.manage(windows);
        app.manage(background::BackgroundTasks::default());
        app.manage(window_prefs::SharedPrefs::default());
        app.manage(notify::Notifier::default());
        app.manage(tray::TrayState::default());
        // Time stands still in tests unless a test moves it (`TestClock`): jobs never cross the visibility
        // and attention thresholds by accident.
        let clock = std::sync::Arc::new(jobs::ManualClock::default());
        app.manage(TestClock(clock.clone()));
        let jobs = jobs::Jobs::new(clock.clone());
        jobs::wire(app.handle(), &jobs);
        app.manage(jobs);
        app.manage(notification_centre::NotificationCentre::new(None, clock));
        // The repository samples count as chosen in the main window (files in a test's own folder do not).
        paths::allow_under(app.handle(), windows::MAIN, &samples());
        browser::manage_state(app.handle(), std::sync::Arc::default());
        app_context::install(app.handle(), app_context::AppContext::for_tests());
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
            .invoke_handler(browser::guard(commands::guard(activity::logged(handler!()))))
            .build(mock_context(noop_assets()))
            .unwrap();
        manage(app, profile_dir)
    }

    /// Like [`test_app_with_commands`], in safe mode (no plugins).
    pub fn test_app_safe_mode() -> tauri::App<MockRuntime> {
        let app = mock_builder()
            .on_window_event(window_manager::on_window_event)
            .invoke_handler(browser::guard(commands::guard(activity::logged(handler!()))))
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
