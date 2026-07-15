//! Tray-only lightweight mode, modeled after cc-switch:
//! destroy the main webview to free UI memory, keep the process in the tray,
//! and recreate the window from tauri.conf on demand.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, WebviewWindowBuilder};

static LIGHTWEIGHT_MODE: AtomicBool = AtomicBool::new(false);

/// When true, CloseRequested must not call prevent_close so destroy()/close() can finish.
static ALLOW_WINDOW_DESTROY: AtomicBool = AtomicBool::new(false);

/// Serialize enter/exit so destroy cannot race a just-recreated window.
static MODE_LOCK: Mutex<()> = Mutex::new(());

pub fn is_lightweight_mode() -> bool {
    LIGHTWEIGHT_MODE.load(Ordering::Acquire)
}

/// Only the short-lived destroy window should skip prevent_close.
/// Do not OR with LIGHTWEIGHT_MODE — that bypassed Ask/Tray/Exit while enter
/// was mid-flight with the main window still visible.
pub fn allow_window_destroy() -> bool {
    ALLOW_WINDOW_DESTROY.load(Ordering::Acquire)
}

pub fn set_allow_window_destroy(allow: bool) {
    ALLOW_WINDOW_DESTROY.store(allow, Ordering::Release)
}

fn restore_existing_window(app: &AppHandle) {
    crate::apply_tray_policy(app, true);

    if let Some(win) = app.get_webview_window("main") {
        #[cfg(target_os = "windows")]
        {
            let _ = win.set_skip_taskbar(false);
        }
        let _ = win.unminimize();
        let _ = win.show();
        let _ = win.set_focus();
    }
}

fn destroy_main_window(app: &AppHandle) -> Result<(), String> {
    set_allow_window_destroy(true);

    if let Some(win) = app.get_webview_window("main") {
        #[cfg(target_os = "windows")]
        {
            let _ = win.set_skip_taskbar(true);
        }

        // Hide Dock before destroy so tray-only mode does not leave a Dock icon.
        crate::apply_tray_policy(app, false);

        if let Err(destroy_error) = win.destroy() {
            if let Err(close_error) = win.close() {
                set_allow_window_destroy(false);
                crate::apply_tray_policy(app, true);
                return Err(format!(
                    "destroy failed: {destroy_error}; close failed: {close_error}"
                ));
            }
        }
    }

    Ok(())
}

/// Recreate main from tauri.conf (same approach as cc-switch).
fn ensure_main_window(app: &AppHandle) -> Result<(), String> {
    if app.get_webview_window("main").is_some() {
        restore_existing_window(app);
        return Ok(());
    }

    // macOS: leave Accessory / hidden-Dock mode BEFORE creating a window.
    // Building a webview while ActivationPolicy::Accessory often fails to
    // produce a visible UI, which makes single-instance / tray restore look broken.
    crate::apply_tray_policy(app, true);

    let window_config = app
        .config()
        .app
        .windows
        .iter()
        .find(|window| window.label == "main")
        .cloned()
        .ok_or_else(|| "main window config not found in tauri.conf".to_string())?;

    // tauri.conf keeps main visible:false for silent/lightweight boot; force
    // visible when restoring from tray-only mode.
    WebviewWindowBuilder::from_config(app, &window_config)
        .map_err(|error| format!("load main window config failed: {error}"))?
        .visible(true)
        .build()
        .map_err(|error| format!("create main window failed: {error}"))?;

    restore_existing_window(app);
    Ok(())
}

/// Enter tray-only mode: destroy webview, mark lightweight, refresh tray.
/// `prepare` runs before destroy (e.g. stop frontend watch / start native watch).
pub fn enter_lightweight_mode<F>(app: &AppHandle, prepare: F) -> Result<(), String>
where
    F: FnOnce(&AppHandle) -> Result<(), String>,
{
    let _guard = MODE_LOCK
        .lock()
        .map_err(|error| format!("lightweight mode lock poisoned: {error}"))?;

    if is_lightweight_mode() && app.get_webview_window("main").is_none() {
        // Already tray-only. prepare is idempotent when a native watch is owned.
        let _ = prepare(app);
        crate::refresh_tray_menu(app)?;
        return Ok(());
    }

    if let Some(tray) = app.tray_by_id(crate::TRAY_ID) {
        tray.set_visible(true).map_err(|error| error.to_string())?;
    }

    // Mark before destroy so ExitRequested (code=None) keeps the process alive.
    LIGHTWEIGHT_MODE.store(true, Ordering::Release);

    // Optional: tell the frontend to stop its shell-owned watch first.
    // Handoff is best-effort; prepare also kills orphans and starts native watch.
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.emit("enter-lightweight-requested", ());
    }

    // Best-effort prepare (watch handover). Destroy UI even if prepare fails.
    let prepare_error = prepare(app).err();
    if let Err(destroy_error) = destroy_main_window(app) {
        // Roll back mode so CloseRequested Ask/Tray/Exit works again and tray
        // does not claim lightweight while the main window is still up.
        LIGHTWEIGHT_MODE.store(false, Ordering::Release);
        set_allow_window_destroy(false);
        let _ = crate::refresh_tray_menu(app);
        return Err(destroy_error);
    }
    set_allow_window_destroy(false);

    if let Some(tray) = app.tray_by_id(crate::TRAY_ID) {
        let _ = tray.set_visible(true);
    }
    crate::refresh_tray_menu(app)?;

    if let Some(error) = prepare_error {
        if let Some(tray) = app.tray_by_id(crate::TRAY_ID) {
            let _ = tray.set_tooltip(Some(format!(
                "AI CLI Complete Notify（轻量模式，监听启动失败: {error}）"
            )));
        }
        return Err(error);
    }

    Ok(())
}

/// Exit tray-only mode: recreate/show main window and clear the flag.
/// Native watch is intentionally left running until the frontend reclaims it.
pub fn exit_lightweight_mode(app: &AppHandle) -> Result<(), String> {
    let _guard = MODE_LOCK
        .lock()
        .map_err(|error| format!("lightweight mode lock poisoned: {error}"))?;

    ensure_main_window(app)?;
    LIGHTWEIGHT_MODE.store(false, Ordering::Release);
    set_allow_window_destroy(false);
    crate::refresh_tray_menu(app)?;
    Ok(())
}

pub fn toggle_lightweight_mode<F>(app: &AppHandle, prepare_enter: F) -> Result<(), String>
where
    F: FnOnce(&AppHandle) -> Result<(), String>,
{
    // Check outside the enter/exit locks; each path takes MODE_LOCK itself.
    if is_lightweight_mode() {
        exit_lightweight_mode(app)
    } else {
        enter_lightweight_mode(app, prepare_enter)
    }
}
