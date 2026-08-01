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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .try_init()
        .ok();

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
                use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};
                // global-hotkey syntax: modifiers first, then Code (e.g. KeyR)
                // Registration fails when another application already owns the
                // combination. That is a missing convenience, not a reason to
                // refuse to start — propagating it here left the app unable to
                // open at all because something else had grabbed Ctrl+Shift+R.
                match "Ctrl+Shift+KeyR".parse::<Shortcut>() {
                    Ok(shortcut) => {
                        let handle = app.handle().clone();
                        if let Err(e) =
                            app.global_shortcut()
                                .on_shortcut(shortcut, move |_app, _sc, event| {
                                    if event.state == ShortcutState::Pressed {
                                        let _ = handle.emit("hotkey://toggle-record", ());
                                    }
                                })
                        {
                            tracing::warn!("global shortcut unavailable: {e}");
                        }
                    }
                    Err(e) => tracing::warn!("invalid global shortcut: {e}"),
                }
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
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_settings,
            commands::save_settings,
            commands::switch_stt_provider,
            commands::switch_llm_provider,
            commands::set_reasoning,
            commands::list_meetings,
            commands::get_meeting,
            commands::get_transcript,
            commands::delete_meeting,
            commands::search_meetings_cmd,
            commands::recorder_status,
            commands::can_record,
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
            commands::poll_live_stt,
            commands::summarize_meeting,
            commands::chat_meeting,
            commands::list_chat,
            commands::import_audio,
            commands::retranscribe,
            commands::export_meeting_cmd,
            commands::list_models_cmd,
            commands::download_model_cmd,
            commands::check_updates_config,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Vesper");
}
