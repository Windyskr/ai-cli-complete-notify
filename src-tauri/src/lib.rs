use serde::Serialize;
use std::{
    env, fs,
    path::PathBuf,
    sync::Mutex,
};
use tauri::{Emitter, Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_shell::process::{CommandChild, CommandEvent};
use tauri_plugin_shell::ShellExt;

const TRAY_ICON: tauri::image::Image<'_> = tauri::include_image!("./icons/icon.png");

#[derive(Clone, Copy)]
struct LaunchState {
    silent_start_requested: bool,
}

struct RuntimeState {
    lightweight: bool,
    native_watch: Option<CommandChild>,
}

impl Default for RuntimeState {
    fn default() -> Self {
        Self {
            lightweight: false,
            native_watch: None,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StartupStatus {
    autostart_enabled: bool,
    autostart_supported: bool,
    silent_start_requested: bool,
    lightweight_mode: bool,
    native_watch_running: bool,
    autostart_error: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CloseBehavior {
    Ask,
    Tray,
    Exit,
}

impl CloseBehavior {
    fn from_config_value(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "tray" => Self::Tray,
            "exit" => Self::Exit,
            _ => Self::Ask,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum UiLanguage {
    Zh,
    En,
}

impl UiLanguage {
    fn from_config_value(value: &str) -> Self {
        let lower = value.trim().to_ascii_lowercase();
        if lower.starts_with("zh") {
            Self::Zh
        } else {
            Self::En
        }
    }

    fn tray_show(self) -> &'static str {
        match self {
            Self::Zh => "显示主界面",
            Self::En => "Show",
        }
    }

    fn tray_lightweight(self) -> &'static str {
        match self {
            Self::Zh => "轻量模式",
            Self::En => "Lightweight Mode",
        }
    }

    fn tray_quit(self) -> &'static str {
        match self {
            Self::Zh => "退出",
            Self::En => "Quit",
        }
    }

    fn tray_tooltip(self, lightweight: bool) -> String {
        match (self, lightweight) {
            (Self::Zh, true) => "AI CLI Complete Notify（轻量模式）".to_string(),
            (Self::Zh, false) => "AI CLI Complete Notify".to_string(),
            (Self::En, true) => "AI CLI Complete Notify (Lightweight)".to_string(),
            (Self::En, false) => "AI CLI Complete Notify".to_string(),
        }
    }
}

fn restore_main_window(app: &tauri::AppHandle) {
    #[cfg(target_os = "macos")]
    let _ = app.show();

    if let Some(win) = app.get_webview_window("main") {
        let _ = win.show();
        let _ = win.unminimize();
        let _ = win.set_focus();
    }
}

fn hide_main_window_to_tray(app: &tauri::AppHandle) -> Result<(), String> {
    if let Some(tray) = app.tray_by_id("main") {
        tray.set_visible(true).map_err(|error| error.to_string())?;
    }

    if let Some(win) = app.get_webview_window("main") {
        win.hide().map_err(|error| error.to_string())?;
    }

    Ok(())
}

fn get_data_dir() -> PathBuf {
    const DATA_DIR_ENV: [&str; 4] = [
        "AI_CLI_COMPLETE_NOTIFY_DATA_DIR",
        "AICLI_COMPLETE_NOTIFY_DATA_DIR",
        "TASKPULSE_DATA_DIR",
        "AI_REMINDER_DATA_DIR",
    ];

    for key in DATA_DIR_ENV {
        if let Ok(value) = env::var(key) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return PathBuf::from(trimmed);
            }
        }
    }

    if let Ok(app_data) = env::var("APPDATA") {
        let trimmed = app_data.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed).join("ai-cli-complete-notify");
        }
    }

    if let Ok(home) = env::var("USERPROFILE").or_else(|_| env::var("HOME")) {
        let trimmed = home.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed).join(".ai-cli-complete-notify");
        }
    }

    PathBuf::from(".ai-cli-complete-notify")
}

fn read_settings_json() -> Option<serde_json::Value> {
    let settings_path = get_data_dir().join("settings.json");
    let bytes = fs::read(&settings_path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn read_close_behavior() -> CloseBehavior {
    read_settings_json()
        .as_ref()
        .and_then(|parsed| parsed.get("ui"))
        .and_then(|ui| ui.get("closeBehavior"))
        .and_then(|value| value.as_str())
        .map(CloseBehavior::from_config_value)
        .unwrap_or(CloseBehavior::Ask)
}

fn read_silent_start_setting() -> bool {
    read_settings_json()
        .as_ref()
        .and_then(|parsed| parsed.get("ui"))
        .and_then(|ui| ui.get("silentStart"))
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

fn read_ui_language() -> UiLanguage {
    read_settings_json()
        .as_ref()
        .and_then(|parsed| parsed.get("ui"))
        .and_then(|ui| ui.get("language"))
        .and_then(|value| value.as_str())
        .map(UiLanguage::from_config_value)
        .unwrap_or(UiLanguage::Zh)
}

fn build_startup_status(app: &tauri::AppHandle, launch_state: LaunchState) -> StartupStatus {
    let runtime = app.state::<Mutex<RuntimeState>>();
    let (lightweight_mode, native_watch_running) = runtime
        .lock()
        .map(|state| (state.lightweight, state.native_watch.is_some()))
        .unwrap_or((false, false));

    match app.autolaunch().is_enabled() {
        Ok(enabled) => StartupStatus {
            autostart_enabled: enabled,
            autostart_supported: true,
            silent_start_requested: launch_state.silent_start_requested,
            lightweight_mode,
            native_watch_running,
            autostart_error: None,
        },
        Err(error) => StartupStatus {
            autostart_enabled: false,
            autostart_supported: false,
            silent_start_requested: launch_state.silent_start_requested,
            lightweight_mode,
            native_watch_running,
            autostart_error: Some(error.to_string()),
        },
    }
}

fn refresh_tray_menu(app: &tauri::AppHandle) -> Result<(), String> {
    let language = read_ui_language();
    let lightweight = app
        .state::<Mutex<RuntimeState>>()
        .lock()
        .map(|state| state.lightweight)
        .unwrap_or(false);

    let tray_menu = tauri::menu::MenuBuilder::new(app)
        .text("show", language.tray_show())
        .text("lightweight", language.tray_lightweight())
        .separator()
        .text("quit", language.tray_quit())
        .build()
        .map_err(|error| error.to_string())?;

    if let Some(tray) = app.tray_by_id("main") {
        tray.set_menu(Some(tray_menu))
            .map_err(|error| error.to_string())?;
        tray.set_tooltip(Some(language.tray_tooltip(lightweight)))
            .map_err(|error| error.to_string())?;
        tray.set_visible(true).map_err(|error| error.to_string())?;
    }

    Ok(())
}

fn stop_native_watch_locked(state: &mut RuntimeState) {
    if let Some(child) = state.native_watch.take() {
        let _ = child.kill();
    }
}

fn start_native_watch(app: &tauri::AppHandle) -> Result<(), String> {
    let runtime = app.state::<Mutex<RuntimeState>>();
    let mut state = runtime.lock().map_err(|error| error.to_string())?;
    if state.native_watch.is_some() {
        return Ok(());
    }

    let sidecar = app
        .shell()
        .sidecar("binaries/ai-reminder")
        .map_err(|error| error.to_string())?
        .args([
            "watch",
            "--sources",
            "all",
            "--interval-ms",
            "1000",
            "--gemini-quiet-ms",
            "3000",
            "--claude-quiet-ms",
            "60000",
        ]);

    let (mut rx, child) = sidecar.spawn().map_err(|error| error.to_string())?;
    state.native_watch = Some(child);
    drop(state);

    // Drain stdout/stderr so the child never blocks on a full pipe.
    tauri::async_runtime::spawn(async move {
        while let Some(event) = rx.recv().await {
            match event {
                CommandEvent::Terminated(_) => break,
                CommandEvent::Error(_) => break,
                _ => {}
            }
        }
    });

    Ok(())
}

fn stop_native_watch(app: &tauri::AppHandle) -> Result<(), String> {
    let runtime = app.state::<Mutex<RuntimeState>>();
    let mut state = runtime.lock().map_err(|error| error.to_string())?;
    stop_native_watch_locked(&mut state);
    Ok(())
}

fn destroy_main_window(app: &tauri::AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_webview_window("main") {
        win.destroy().map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn ensure_main_window(app: &tauri::AppHandle) -> Result<(), String> {
    if app.get_webview_window("main").is_some() {
        restore_main_window(app);
        return Ok(());
    }

    WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
        .title("AI CLI Complete Notify")
        .inner_size(1280.0, 860.0)
        .min_inner_size(1040.0, 720.0)
        .center()
        .visible(true)
        .build()
        .map_err(|error| error.to_string())?;

    restore_main_window(app);
    Ok(())
}

fn enter_lightweight_mode_impl(app: &tauri::AppHandle) -> Result<(), String> {
    if let Some(tray) = app.tray_by_id("main") {
        tray.set_visible(true).map_err(|error| error.to_string())?;
    }

    // Keep completion monitoring alive after the webview is destroyed.
    start_native_watch(app)?;
    destroy_main_window(app)?;

    {
        let runtime = app.state::<Mutex<RuntimeState>>();
        let mut state = runtime.lock().map_err(|error| error.to_string())?;
        state.lightweight = true;
    }

    refresh_tray_menu(app)?;
    Ok(())
}

fn exit_lightweight_mode_impl(app: &tauri::AppHandle) -> Result<(), String> {
    // Recreate the UI first. Keep any native watch running until the frontend
    // auto-starts and calls stop_native_watch_command, so there is no gap.
    ensure_main_window(app)?;

    {
        let runtime = app.state::<Mutex<RuntimeState>>();
        let mut state = runtime.lock().map_err(|error| error.to_string())?;
        state.lightweight = false;
    }

    refresh_tray_menu(app)?;
    Ok(())
}

fn request_enter_lightweight_mode(app: &tauri::AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        // Ask the frontend to stop its shell-owned watch first, then confirm.
        let _ = win.emit("enter-lightweight-requested", ());
        return;
    }

    let _ = enter_lightweight_mode_impl(app);
}

#[tauri::command]
fn get_startup_status(
    app: tauri::AppHandle,
    launch_state: tauri::State<'_, LaunchState>,
) -> StartupStatus {
    build_startup_status(&app, *launch_state.inner())
}

#[tauri::command]
fn set_autostart_enabled(app: tauri::AppHandle, enabled: bool) -> Result<bool, String> {
    let autostart = app.autolaunch();
    if enabled {
        autostart.enable().map_err(|error| error.to_string())?;
    } else {
        autostart.disable().map_err(|error| error.to_string())?;
    }
    autostart.is_enabled().map_err(|error| error.to_string())
}

#[tauri::command]
fn hide_to_tray(app: tauri::AppHandle) -> Result<(), String> {
    hide_main_window_to_tray(&app)
}

#[tauri::command]
fn enter_lightweight_mode(app: tauri::AppHandle) -> Result<(), String> {
    enter_lightweight_mode_impl(&app)
}

#[tauri::command]
fn stop_native_watch_command(app: tauri::AppHandle) -> Result<(), String> {
    stop_native_watch(&app)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            let _ = exit_lightweight_mode_impl(app);
        }))
        .plugin(
            tauri_plugin_autostart::Builder::new()
                .args(["--silent-start"])
                .build(),
        )
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_notification::init())
        .invoke_handler(tauri::generate_handler![
            get_startup_status,
            set_autostart_enabled,
            hide_to_tray,
            enter_lightweight_mode,
            stop_native_watch_command
        ])
        .setup(|app| {
            let launch_state = LaunchState {
                silent_start_requested: std::env::args().any(|arg| arg == "--silent-start"),
            };
            let should_stay_hidden =
                launch_state.silent_start_requested || read_silent_start_setting();

            app.manage(launch_state);
            app.manage(Mutex::new(RuntimeState::default()));

            let language = read_ui_language();
            let tray_menu = tauri::menu::MenuBuilder::new(app)
                .text("show", language.tray_show())
                .text("lightweight", language.tray_lightweight())
                .separator()
                .text("quit", language.tray_quit())
                .build()?;

            let tray = tauri::tray::TrayIconBuilder::with_id("main")
                .icon(TRAY_ICON.clone())
                .menu(&tray_menu)
                .tooltip(language.tray_tooltip(false))
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "show" => {
                        let _ = exit_lightweight_mode_impl(app);
                    }
                    "lightweight" => {
                        request_enter_lightweight_mode(app);
                    }
                    "quit" => {
                        let _ = stop_native_watch(app);
                        app.exit(0);
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let tauri::tray::TrayIconEvent::Click {
                        button: tauri::tray::MouseButton::Left,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        let _ = exit_lightweight_mode_impl(&app);
                    }
                })
                .build(app)?;

            let _ = tray.set_visible(true);

            if !should_stay_hidden {
                restore_main_window(app.handle());
            }

            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();

                match read_close_behavior() {
                    CloseBehavior::Tray => {
                        let _ = hide_main_window_to_tray(window.app_handle());
                    }
                    CloseBehavior::Exit => {
                        let _ = stop_native_watch(window.app_handle());
                        window.app_handle().exit(0);
                    }
                    CloseBehavior::Ask => {
                        let _ = window.emit("app-close-requested", ());
                    }
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen {
                has_visible_windows,
                ..
            } = event
            {
                if !has_visible_windows {
                    let _ = exit_lightweight_mode_impl(app);
                }
            }

            #[cfg(not(target_os = "macos"))]
            let _ = (app, event);
        });
}
