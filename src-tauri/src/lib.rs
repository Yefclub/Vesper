mod audio;
mod commands;
mod db;
mod domain;
mod llm;
mod models;
mod paths;
mod secrets;
mod stt;

use commands::AppState;
use std::sync::Arc;
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Emitter, Manager,
};

/// `RUST_LOG` if it is set and parses, `warn` otherwise.
fn default_log_filter() -> tracing_subscriber::EnvFilter {
    tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn"))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // A release build has no console — `main.rs` sets
    // `windows_subsystem = "windows"` — so until now every `tracing::warn!` in
    // the shipped app went to a stdout that does not exist. The refused global
    // shortcut below is only the first one anybody noticed.
    match tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("vesper")
        .filename_suffix("log")
        .max_log_files(5)
        .build(paths::logs_dir())
    {
        Ok(file) => {
            use tracing_subscriber::fmt::writer::MakeWriterExt;
            tracing_subscriber::fmt()
                // `warn` by default. Daily rotation caps the number of files at
                // five, but nothing caps the size of the one being written, and a
                // chatty level on a machine left running for a day is the only
                // way this grows without a bound. The point of the file is the
                // failure nobody saw, not a narrative.
                //
                // `RUST_LOG` overrides it, which is what makes the `info` lines
                // reachable — the one naming the compute backend a model was
                // actually loaded onto answers "why is this slow", and a line
                // nobody can turn on answers nothing.
                .with_env_filter(default_log_filter())
                // Colour codes are noise in a file, and the console keeps the
                // same lines either way.
                .with_ansi(false)
                .with_writer(file.and(std::io::stdout))
                .try_init()
                .ok();
        }
        // A log directory that cannot be written is not a reason to lose the
        // console too.
        Err(e) => {
            tracing_subscriber::fmt()
                .with_env_filter(default_log_filter())
                .try_init()
                .ok();
            tracing::warn!("log file unavailable, console only: {e}");
        }
    }

    let state = match AppState::new() {
        Ok(s) => Arc::new(s),
        Err(e) => {
            eprintln!("failed to init app state: {e}");
            panic!("failed to init app state: {e}");
        }
    };

    tauri::Builder::default()
        // A second copy would open the same SQLite file and fight over the audio
        // device. Focus the window that is already running instead.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .manage(state)
        .setup(|app| {
            #[cfg(desktop)]
            {
                // Registration fails when another application already owns the
                // combination. That is a missing convenience, not a reason to
                // refuse to start — propagating it here left the app unable to
                // open at all because something else had grabbed Ctrl+Shift+R.
                // Swallowing it was the other extreme: the window went on
                // advertising a key the OS had refused. Keep it, and report it.
                // Whatever the user chose, or the default when the stored value
                // is one this build no longer offers.
                let accelerator = {
                    let state = app.state::<Arc<AppState>>();
                    let stored = state.settings.lock().record_shortcut.clone();
                    domain::shortcut::chosen_or_default(&stored)
                };
                let registered = commands::register_record_shortcut(app.handle(), accelerator);
                if let Err(e) = &registered {
                    tracing::warn!("global shortcut unavailable: {e}");
                }
                app.manage(std::sync::Mutex::new(domain::shortcut::status_from(
                    registered,
                    accelerator,
                )));
            }

            let show_i = MenuItem::with_id(app, "show", "Show Vesper", true, None::<&str>)?;
            let rec_i = MenuItem::with_id(
                app,
                "toggle_record",
                "Start/Stop Recording",
                true,
                None::<&str>,
            )?;
            let quit_i = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show_i, &rec_i, &quit_i])?;

            // The only tray icon. `tauri.conf.json` used to carry a `trayIcon`
            // block as well, and Tauri builds one from that at startup — so the
            // notification area held two Vespers, the config's with no menu at
            // all and this one with all the behaviour. The config block is gone;
            // putting it back puts the second icon back.
            let _tray = TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&menu)
                .tooltip("Vesper")
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "quit" => app.exit(0),
                    "show" => {
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.set_focus();
                            let _ = w.maximize();
                        }
                    }
                    "toggle_record" => {
                        let _ = app.emit("hotkey://toggle-record", ());
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.set_focus();
                            let _ = w.maximize();
                        }
                    }
                })
                .build(app)?;

            // Always open maximized (tela cheia de trabalho)
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.maximize();
                // `decorations` stays true, so without this a light app sits
                // under a dark native title bar. Never `?`: a window that fails
                // to theme must still appear.
                let theme = app.state::<Arc<AppState>>().settings.lock().theme.clone();
                let _ = w.set_theme(Some(if theme == "dark" {
                    tauri::Theme::Dark
                } else {
                    tauri::Theme::Light
                }));

                // There is no `WindowEvent::Minimized` in Tauri. Windows reports
                // a minimize as `Resized(0,0)` followed by `Focused(false)`, so
                // both are listened for and `is_minimized()` — which is
                // `IsIconic` underneath — is the authority rather than the
                // payload.
                let handle = app.handle().clone();
                w.on_window_event(move |event| {
                    if matches!(
                        event,
                        tauri::WindowEvent::Resized(_) | tauri::WindowEvent::Focused(_)
                    ) {
                        let state = handle.state::<Arc<AppState>>();
                        commands::sync_overlay_for(&handle, &state);
                    }
                    // Close means hide, when the user asked for that. The window
                    // goes away and the tray icon is what brings it back — the
                    // recording, if there is one, carries on, which is the whole
                    // point of the setting.
                    //
                    // A recording is NOT a reason to override this. Somebody who
                    // turned the switch on has said what they want a close to
                    // mean, and a close that quits anyway during the one state
                    // where staying matters would be the setting failing at its
                    // own job.
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        let state = handle.state::<Arc<AppState>>();
                        let to_tray = state.settings.lock().close_to_tray;
                        if to_tray {
                            api.prevent_close();
                            if let Some(w) = handle.get_webview_window("main") {
                                let _ = w.hide();
                            }
                            // Hidden is not focused, so the card has to be
                            // reconsidered — this is exactly the case it exists
                            // for.
                            commands::sync_overlay_for(&handle, &state);
                        }
                    }
                    // A hidden window is still a window and Tauri only exits once
                    // every one is destroyed, so closing the main window has to
                    // take the card with it or Vesper keeps running invisibly.
                    if matches!(event, tauri::WindowEvent::Destroyed) {
                        if let Some(o) = handle.get_webview_window("overlay") {
                            let _ = o.close();
                        }
                    }
                });
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_settings,
            commands::save_settings,
            commands::set_theme,
            commands::switch_stt_provider,
            commands::switch_llm_provider,
            commands::set_reasoning,
            commands::list_meetings,
            commands::get_meeting,
            commands::get_transcript,
            commands::list_action_items,
            commands::add_action_item,
            commands::update_action_item,
            commands::delete_action_item,
            commands::edit_transcript_segment,
            commands::delete_meeting,
            commands::search_meetings_cmd,
            commands::recorder_status,
            commands::can_record,
            commands::shortcut_status,
            commands::list_audio_devices_cmd,
            commands::get_i18n_catalog,
            commands::translate_key,
            commands::get_capabilities,
            commands::list_openrouter_stt_models,
            commands::list_openrouter_llm_models,
            commands::complete_onboarding,
            commands::start_recording,
            commands::pause_recording,
            commands::resume_recording,
            commands::stop_recording,
            commands::summarize_meeting,
            commands::list_summary_versions,
            commands::refine_summary_section,
            commands::restore_summary_version,
            commands::set_overlay_expanded,
            commands::keep_recording,
            commands::set_record_shortcut,
            commands::set_modal_open,
            commands::chat_meeting,
            commands::list_chat,
            commands::add_context_note,
            commands::list_context_notes,
            commands::delete_context_note,
            commands::import_audio,
            commands::retranscribe,
            commands::rename_meeting,
            commands::set_speaker_names,
            commands::export_meeting_cmd,
            commands::export_all_cmd,
            commands::wipe_all_cmd,
            commands::suggested_export_name,
            commands::list_models_cmd,
            commands::download_model_cmd,
            commands::check_updates_config,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Vesper");
}
