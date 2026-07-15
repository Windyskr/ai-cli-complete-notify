mod lightweight;

use serde::Serialize;
use std::{
    env, fs,
    path::PathBuf,
    process::{Child as StdChild, Command as StdCommand},
    sync::Mutex,
};
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use tauri::{Emitter, Manager};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_shell::process::{CommandChild, CommandEvent};
use tauri_plugin_shell::ShellExt;

/// Hide console windows when spawning helper processes on Windows.
/// Without this, Windows 11 often opens Windows Terminal for cmd/console apps,
/// which looks like a blank popup titled with the app working directory.
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[cfg(target_os = "windows")]
fn hide_console(cmd: &mut StdCommand) -> &mut StdCommand {
    cmd.creation_flags(CREATE_NO_WINDOW)
}

#[cfg(not(target_os = "windows"))]
fn hide_console(cmd: &mut StdCommand) -> &mut StdCommand {
    cmd
}

const TRAY_ICON: tauri::image::Image<'_> = tauri::include_image!("./icons/icon.png");
pub(crate) const TRAY_ID: &str = "main";

#[derive(Clone, Copy)]
struct LaunchState {
    silent_start_requested: bool,
}

struct RuntimeState {
    /// When true, ExitRequested is allowed to finish (tray Quit / explicit exit).
    allow_app_exit: bool,
    /// Bumped on every stop so in-flight start paths discard their child.
    watch_generation: u64,
    /// Preferred Tauri shell sidecar watch, tagged with the generation at store time.
    native_watch: Option<(u64, CommandChild)>,
    /// Portable/fallback std::process watch (kept so we can kill by handle).
    fallback_watch: Option<(u64, StdChild)>,
}

impl Default for RuntimeState {
    fn default() -> Self {
        Self {
            allow_app_exit: false,
            watch_generation: 0,
            native_watch: None,
            fallback_watch: None,
        }
    }
}

impl RuntimeState {
    fn has_owned_watch(&self) -> bool {
        self.native_watch.is_some() || self.fallback_watch.is_some()
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

/// Classify ExitRequested the same way cc-switch does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExitRequestAction {
    /// code is None: runtime auto-exit after last window destroyed → stay in tray.
    StayInTray,
    /// other Some(code): user/app.exit → cleanup and exit.
    CleanupAndExit,
}

fn classify_exit_request(code: Option<i32>) -> ExitRequestAction {
    match code {
        None => ExitRequestAction::StayInTray,
        Some(_) => ExitRequestAction::CleanupAndExit,
    }
}

fn restore_main_window(app: &tauri::AppHandle) {
    apply_tray_policy(app, true);

    if let Some(win) = app.get_webview_window("main") {
        #[cfg(target_os = "windows")]
        {
            let _ = win.set_skip_taskbar(false);
        }
        let _ = win.show();
        let _ = win.unminimize();
        let _ = win.set_focus();
    }
}

fn hide_main_window_to_tray(app: &tauri::AppHandle) -> Result<(), String> {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        tray.set_visible(true).map_err(|error| error.to_string())?;
    }

    if let Some(win) = app.get_webview_window("main") {
        #[cfg(target_os = "windows")]
        {
            let _ = win.set_skip_taskbar(true);
        }
        win.hide().map_err(|error| error.to_string())?;
    }

    // Close-to-tray should also drop the Dock icon (cc-switch style).
    apply_tray_policy(app, false);

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

fn read_lightweight_start_setting() -> bool {
    read_settings_json()
        .as_ref()
        .and_then(|parsed| parsed.get("ui"))
        .and_then(|ui| ui.get("lightweightStart"))
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
    let native_watch_running = runtime
        .lock()
        .map(|state| state.has_owned_watch())
        .unwrap_or(false);

    match app.autolaunch().is_enabled() {
        Ok(enabled) => StartupStatus {
            autostart_enabled: enabled,
            autostart_supported: true,
            silent_start_requested: launch_state.silent_start_requested,
            lightweight_mode: lightweight::is_lightweight_mode(),
            native_watch_running,
            autostart_error: None,
        },
        Err(error) => StartupStatus {
            autostart_enabled: false,
            autostart_supported: false,
            silent_start_requested: launch_state.silent_start_requested,
            lightweight_mode: lightweight::is_lightweight_mode(),
            native_watch_running,
            autostart_error: Some(error.to_string()),
        },
    }
}

pub(crate) fn refresh_tray_menu(app: &tauri::AppHandle) -> Result<(), String> {
    let language = read_ui_language();
    let lightweight = lightweight::is_lightweight_mode();

    let show_item = tauri::menu::MenuItem::with_id(
        app,
        "show_main",
        language.tray_show(),
        true,
        None::<&str>,
    )
    .map_err(|error| error.to_string())?;

    // CheckMenuItem toggle — same UX as cc-switch.
    let lightweight_item = tauri::menu::CheckMenuItem::with_id(
        app,
        "lightweight_mode",
        language.tray_lightweight(),
        true,
        lightweight,
        None::<&str>,
    )
    .map_err(|error| error.to_string())?;

    let quit_item = tauri::menu::MenuItem::with_id(
        app,
        "quit",
        language.tray_quit(),
        true,
        None::<&str>,
    )
    .map_err(|error| error.to_string())?;

    let tray_menu = tauri::menu::MenuBuilder::new(app)
        .item(&show_item)
        .item(&lightweight_item)
        .separator()
        .item(&quit_item)
        .build()
        .map_err(|error| error.to_string())?;

    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        tray.set_menu(Some(tray_menu))
            .map_err(|error| error.to_string())?;
        tray.set_tooltip(Some(language.tray_tooltip(lightweight)))
            .map_err(|error| error.to_string())?;
        tray.set_visible(true).map_err(|error| error.to_string())?;
    }

    Ok(())
}

fn stop_native_watch_locked(state: &mut RuntimeState) {
    // Invalidate any in-flight start_native_watch that already dropped the lock.
    state.watch_generation = state.watch_generation.wrapping_add(1);

    if let Some((_gen, child)) = state.native_watch.take() {
        let _ = child.kill();
    }
    if let Some((_gen, mut child)) = state.fallback_watch.take() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// Best-effort: stop shell-plugin watches this app does not hold a Child for.
/// Prefer owned-handle kill; this is only for UI-spawned leftovers.
fn kill_orphaned_ai_reminder_watchers() {
    #[cfg(target_os = "windows")]
    {
        // Must use CREATE_NO_WINDOW: plain cmd/wmic opens Windows Terminal on Win11
        // with a blank tab titled like the working directory path.
        // Match ai-reminder*.exe (portable ai-reminder.exe and any triple-named residual).
        let mut cmd = StdCommand::new("powershell.exe");
        hide_console(&mut cmd)
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-WindowStyle",
                "Hidden",
                "-Command",
                "Get-CimInstance Win32_Process | Where-Object { $_.Name -like 'ai-reminder*.exe' -and $_.CommandLine -match '(\\s|^)watch(\\s|$)' } | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }",
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .ok();
    }

    // macOS/Linux: the packaged sidecar is a shell wrapper that execs
    // `node .../ai-reminder.js watch ...`. BSD pkill uses basic regex and does
    // not understand `\s`, so match concrete argv fragments only (avoid broad
    // `ai-reminder-.* watch` which can hit unrelated tools).
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        for pattern in ["ai-reminder.js watch", "ai-reminder watch"] {
            let mut cmd = StdCommand::new("pkill");
            hide_console(&mut cmd)
                .args(["-f", pattern])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .ok();
        }
    }
}

/// macOS: hide/show Dock + switch ActivationPolicy (same idea as cc-switch).
/// Accessory keeps a tray-only process out of the Dock after webview destroy.
#[cfg(target_os = "macos")]
fn apply_tray_policy(app: &tauri::AppHandle, dock_visible: bool) {
    use tauri::ActivationPolicy;

    let desired = if dock_visible {
        ActivationPolicy::Regular
    } else {
        ActivationPolicy::Accessory
    };

    let _ = app.set_dock_visibility(dock_visible);
    let _ = app.set_activation_policy(desired);
}

#[cfg(not(target_os = "macos"))]
fn apply_tray_policy(_app: &tauri::AppHandle, _dock_visible: bool) {}

fn start_native_watch(app: &tauri::AppHandle) -> Result<(), String> {
    // Capture generation under the lock, spawn outside, then store only if the
    // generation is unchanged (stop_native_watch bumps it and kills owned children).
    let runtime = app.state::<Mutex<RuntimeState>>();
    let state = runtime.lock().map_err(|error| error.to_string())?;
    if state.has_owned_watch() {
        return Ok(());
    }
    let generation = state.watch_generation;
    drop(state);

    let watch_args = [
        "watch",
        "--sources",
        "all",
        "--interval-ms",
        "1000",
        "--gemini-quiet-ms",
        "3000",
        "--claude-quiet-ms",
        "60000",
    ];

    // Preferred: Tauri sidecar (same path the UI uses).
    let sidecar_result = app
        .shell()
        .sidecar("binaries/ai-reminder")
        .map_err(|error| error.to_string())
        .and_then(|cmd| {
            cmd.args(watch_args)
                .spawn()
                .map_err(|error| error.to_string())
        });

    if let Ok((mut rx, child)) = sidecar_result {
        let mut state = runtime.lock().map_err(|error| error.to_string())?;
        if state.watch_generation != generation || state.has_owned_watch() {
            // Stop won the race — discard this spawn.
            let _ = child.kill();
            return Ok(());
        }
        state.native_watch = Some((generation, child));
        drop(state);

        let app_handle = app.clone();
        tauri::async_runtime::spawn(async move {
            while let Some(event) = rx.recv().await {
                match event {
                    CommandEvent::Terminated(_) => break,
                    CommandEvent::Error(_) => break,
                    _ => {}
                }
            }
            // Reflect process death so prepare/start can respawn a healthy watch.
            if let Ok(mut state) = app_handle.state::<Mutex<RuntimeState>>().lock() {
                if let Some((gen, _)) = state.native_watch.as_ref() {
                    if *gen == generation {
                        state.native_watch = None;
                    }
                }
            }
        });
        return Ok(());
    }

    // Fallback for portable layouts: spawn ai-reminder next to the main exe.
    let exe = env::current_exe().map_err(|error| error.to_string())?;
    let dir = exe
        .parent()
        .ok_or_else(|| "cannot resolve executable directory".to_string())?;
    let candidates = [
        dir.join("ai-reminder.exe"),
        dir.join("ai-reminder-x86_64-pc-windows-msvc.exe"),
        dir.join("ai-reminder"),
        dir.join("ai-reminder-x86_64-apple-darwin"),
        dir.join("ai-reminder-aarch64-apple-darwin"),
        dir.join("ai-reminder-x86_64-unknown-linux-gnu"),
        dir.join("ai-reminder-aarch64-unknown-linux-gnu"),
    ];
    let sidecar_path = candidates
        .into_iter()
        .find(|path| path.exists())
        .ok_or_else(|| {
            format!(
                "failed to start native watch: sidecar missing next to {}",
                dir.display()
            )
        })?;

    let mut cmd = StdCommand::new(&sidecar_path);
    let mut child = hide_console(&mut cmd)
        .args(watch_args)
        .current_dir(dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|error| {
            format!(
                "failed to spawn {}: {}",
                sidecar_path.display(),
                error
            )
        })?;

    let mut state = runtime.lock().map_err(|error| error.to_string())?;
    if state.watch_generation != generation || state.has_owned_watch() {
        let _ = child.kill();
        let _ = child.wait();
        return Ok(());
    }
    // Keep the Child so stop can kill by handle.
    state.fallback_watch = Some((generation, child));
    Ok(())
}

fn stop_native_watch(app: &tauri::AppHandle) -> Result<(), String> {
    let runtime = app.state::<Mutex<RuntimeState>>();
    let mut state = runtime.lock().map_err(|error| error.to_string())?;
    // Bump generation + kill owned handles under the lock first so a concurrent
    // start cannot store a child that kill_orphaned would then murder while
    // RuntimeState still claims ownership.
    stop_native_watch_locked(&mut state);
    drop(state);
    kill_orphaned_ai_reminder_watchers();
    Ok(())
}

fn set_allow_app_exit(app: &tauri::AppHandle, allow: bool) {
    if let Ok(mut state) = app.state::<Mutex<RuntimeState>>().lock() {
        state.allow_app_exit = allow;
    }
}

/// Hide tray icon before hard exit so Windows does not leave a ghost icon.
fn remove_tray_icon_before_exit(app: &tauri::AppHandle) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_visible(false);
    }
}

fn request_app_exit(app: &tauri::AppHandle) {
    set_allow_app_exit(app, true);
    let _ = stop_native_watch(app);
    remove_tray_icon_before_exit(app);
    app.exit(0);
}

/// Prepare watch ownership for lightweight mode (kill UI watch, start native).
fn prepare_lightweight_watch(app: &tauri::AppHandle) -> Result<(), String> {
    // If Rust already owns a healthy watch, leave it alone. Re-entering
    // lightweight mode (boot safety path / tray toggle race) must not kill and
    // respawn, or macOS ends up with multiple orphaned watchers.
    // Death of the child clears ownership (see start_native_watch reaper).
    let already_running = app
        .state::<Mutex<RuntimeState>>()
        .lock()
        .map(|state| state.has_owned_watch())
        .unwrap_or(false);
    if already_running {
        return Ok(());
    }

    kill_orphaned_ai_reminder_watchers();
    start_native_watch(app)
}

fn handle_tray_menu_event(app: &tauri::AppHandle, event_id: &str) {
    match event_id {
        "show_main" => {
            if let Err(error) = lightweight::exit_lightweight_mode(app) {
                eprintln!("[lightweight] tray show_main failed: {error}");
            }
            restore_main_window(app);
        }
        "lightweight_mode" => {
            // CheckMenuItem toggle: enter or exit based on current flag.
            // Always notify frontend first when entering so it can stop shell watch.
            let _ = lightweight::toggle_lightweight_mode(app, prepare_lightweight_watch);
        }
        "quit" => {
            request_app_exit(app);
        }
        _ => {}
    }
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
    lightweight::enter_lightweight_mode(&app, prepare_lightweight_watch)
}

#[tauri::command]
fn exit_lightweight_mode(app: tauri::AppHandle) -> Result<(), String> {
    lightweight::exit_lightweight_mode(&app)
}

#[tauri::command]
fn is_lightweight_mode() -> bool {
    lightweight::is_lightweight_mode()
}

#[tauri::command]
fn stop_native_watch_command(app: tauri::AppHandle) -> Result<(), String> {
    stop_native_watch(&app)
}

#[tauri::command]
fn request_app_exit_command(app: tauri::AppHandle) {
    request_app_exit(&app);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Err(error) = lightweight::exit_lightweight_mode(app) {
                eprintln!("[lightweight] single-instance restore failed: {error}");
            }
            // Even if we were only hidden (not destroyed), force show.
            restore_main_window(app);
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
            exit_lightweight_mode,
            is_lightweight_mode,
            stop_native_watch_command,
            request_app_exit_command
        ])
        .setup(|app| {
            let launch_state = LaunchState {
                silent_start_requested: std::env::args().any(|arg| arg == "--silent-start"),
            };
            // Boot straight into tray-only lightweight mode when configured.
            // This takes precedence over silentStart (hide window only).
            let lightweight_start = read_lightweight_start_setting();
            let should_stay_hidden = launch_state.silent_start_requested
                || read_silent_start_setting()
                || lightweight_start;

            app.manage(launch_state);
            app.manage(Mutex::new(RuntimeState::default()));

            let language = read_ui_language();
            let show_item = tauri::menu::MenuItem::with_id(
                app,
                "show_main",
                language.tray_show(),
                true,
                None::<&str>,
            )?;
            let lightweight_item = tauri::menu::CheckMenuItem::with_id(
                app,
                "lightweight_mode",
                language.tray_lightweight(),
                true,
                lightweight_start,
                None::<&str>,
            )?;
            let quit_item = tauri::menu::MenuItem::with_id(
                app,
                "quit",
                language.tray_quit(),
                true,
                None::<&str>,
            )?;
            let tray_menu = tauri::menu::MenuBuilder::new(app)
                .item(&show_item)
                .item(&lightweight_item)
                .separator()
                .item(&quit_item)
                .build()?;

            let tray = tauri::tray::TrayIconBuilder::with_id(TRAY_ID)
                .icon(TRAY_ICON.clone())
                .menu(&tray_menu)
                .tooltip(language.tray_tooltip(lightweight_start))
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| {
                    handle_tray_menu_event(app, event.id().as_ref());
                })
                .on_tray_icon_event(|tray, event| {
                    if let tauri::tray::TrayIconEvent::Click {
                        button: tauri::tray::MouseButton::Left,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        if let Err(error) = lightweight::exit_lightweight_mode(app) {
                            eprintln!("[lightweight] tray click restore failed: {error}");
                        }
                        restore_main_window(app);
                    }
                })
                .build(app)?;

            let _ = tray.set_visible(true);

            if lightweight_start {
                // Destroy the webview and keep a Rust-owned watch without loading UI.
                let _ = lightweight::enter_lightweight_mode(app.handle(), prepare_lightweight_watch);
            } else if !should_stay_hidden {
                restore_main_window(app.handle());
            }

            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                // Lightweight destroy()/close() also raises CloseRequested.
                // If we always prevent_close, tray lightweight mode appears broken.
                if lightweight::allow_window_destroy() {
                    return;
                }

                api.prevent_close();

                match read_close_behavior() {
                    CloseBehavior::Tray => {
                        // Close-to-tray is hide only (cc-switch style), not destroy.
                        let _ = hide_main_window_to_tray(window.app_handle());
                    }
                    CloseBehavior::Exit => {
                        request_app_exit(window.app_handle());
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
            match event {
                // Destroying the last webview triggers ExitRequested with no
                // exit code. Keep the process/tray alive (cc-switch pattern).
                // Explicit app.exit(code) (tray Quit / frontend exit) carries a
                // code and is allowed through.
                tauri::RunEvent::ExitRequested { api, code, .. } => {
                    let allow_flag = app
                        .state::<Mutex<RuntimeState>>()
                        .lock()
                        .map(|state| state.allow_app_exit)
                        .unwrap_or(false);

                    match classify_exit_request(code) {
                        ExitRequestAction::StayInTray if !allow_flag => {
                            api.prevent_exit();
                            if let Some(tray) = app.tray_by_id(TRAY_ID) {
                                let _ = tray.set_visible(true);
                            }
                        }
                        ExitRequestAction::CleanupAndExit | ExitRequestAction::StayInTray => {
                            // User quit path: ensure watch is stopped and tray is cleared.
                            // app.exit already set allow_app_exit; just let it proceed.
                            let _ = stop_native_watch(app);
                            remove_tray_icon_before_exit(app);
                        }
                    }
                }
                #[cfg(target_os = "macos")]
                tauri::RunEvent::Reopen {
                    has_visible_windows,
                    ..
                } => {
                    if !has_visible_windows {
                        if let Err(error) = lightweight::exit_lightweight_mode(app) {
                            eprintln!("[lightweight] reopen restore failed: {error}");
                        }
                        restore_main_window(app);
                    }
                }
                _ => {}
            }
        });
}

#[cfg(test)]
mod tests {
    use super::{classify_exit_request, ExitRequestAction};

    #[test]
    fn no_code_keeps_app_alive_in_tray() {
        assert_eq!(classify_exit_request(None), ExitRequestAction::StayInTray);
    }

    #[test]
    fn some_code_is_user_exit() {
        assert_eq!(
            classify_exit_request(Some(0)),
            ExitRequestAction::CleanupAndExit
        );
    }
}
