import { invoke } from '@tauri-apps/api/core';

export function hideToTray() {
  return invoke<void>('hide_to_tray');
}

/** Destroy the main webview and keep native watch alive in tray-only mode. */
export function enterLightweightMode() {
  return invoke<void>('enter_lightweight_mode');
}

/** Recreate/show the main window and leave tray-only mode. */
export function exitLightweightMode() {
  return invoke<void>('exit_lightweight_mode');
}

export function isLightweightMode() {
  return invoke<boolean>('is_lightweight_mode');
}

/** Stop the Rust-owned watch process before the frontend takes over. */
export function stopNativeWatch() {
  return invoke<void>('stop_native_watch_command');
}

/** Unified quit path: stop native watch, clear tray, then exit. */
export function requestAppExit() {
  return invoke<void>('request_app_exit_command');
}
