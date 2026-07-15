const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');

const root = path.join(__dirname, '..');

test('close dialog hide action delegates to the native hide_to_tray command', () => {
  const appSource = fs.readFileSync(path.join(root, 'src-ui', 'App.tsx'), 'utf8');
  const rustSource = fs.readFileSync(path.join(root, 'src-tauri', 'src', 'lib.rs'), 'utf8');

  assert.match(appSource, /hideToTray\(\)/);
  assert.doesNotMatch(appSource, /if \(action === 'tray'\) \{\s*await appWindow\.hide\(\);/);
  assert.match(rustSource, /fn hide_to_tray\(/);
  assert.match(rustSource, /hide_to_tray/);
});

test('macOS tray hide keeps the app reopenable from dock and menu bar', () => {
  const rustSource = fs.readFileSync(path.join(root, 'src-tauri', 'src', 'lib.rs'), 'utf8');
  const lightweightSource = fs.readFileSync(
    path.join(root, 'src-tauri', 'src', 'lightweight.rs'),
    'utf8',
  );

  assert.doesNotMatch(rustSource, /app\.hide\(\)/);
  assert.doesNotMatch(rustSource, /button_state:\s*tauri::tray::MouseButtonState::Up/);
  assert.match(rustSource, /RunEvent::Reopen/);
  assert.match(lightweightSource, /exit_lightweight_mode/);
});

test('tray lightweight mode follows cc-switch destroy + prevent_exit pattern', () => {
  const appSource = fs.readFileSync(path.join(root, 'src-ui', 'App.tsx'), 'utf8');
  const windowSource = fs.readFileSync(path.join(root, 'src-ui', 'lib', 'window.ts'), 'utf8');
  const watchSource = fs.readFileSync(path.join(root, 'src-ui', 'hooks', 'useWatch.ts'), 'utf8');
  const rustSource = fs.readFileSync(path.join(root, 'src-tauri', 'src', 'lib.rs'), 'utf8');
  const lightweightSource = fs.readFileSync(
    path.join(root, 'src-tauri', 'src', 'lightweight.rs'),
    'utf8',
  );

  // Dedicated module (cc-switch style).
  assert.match(rustSource, /mod lightweight;/);
  assert.match(lightweightSource, /fn enter_lightweight_mode/);
  assert.match(lightweightSource, /fn exit_lightweight_mode/);
  assert.match(lightweightSource, /fn is_lightweight_mode/);
  assert.match(lightweightSource, /\.destroy\(\)/);
  assert.match(lightweightSource, /WebviewWindowBuilder::from_config/);
  assert.match(lightweightSource, /AtomicBool/);

  // Tray uses CheckMenuItem toggle, not a one-shot text item only.
  assert.match(rustSource, /CheckMenuItem::with_id/);
  assert.match(rustSource, /"lightweight_mode"/);
  assert.match(rustSource, /toggle_lightweight_mode|enter_lightweight_mode/);

  // ExitRequested: code=None stays in tray.
  assert.match(rustSource, /ExitRequested/);
  assert.match(rustSource, /prevent_exit\(\)/);
  assert.match(rustSource, /classify_exit_request|StayInTray/);

  // Close-to-tray remains hide (not destroy).
  assert.match(rustSource, /hide_main_window_to_tray/);

  // Helper processes must not open a visible Windows Terminal / console popup.
  assert.match(rustSource, /CREATE_NO_WINDOW/);
  assert.match(rustSource, /hide_console/);

  // Watch handover kept for this app's architecture.
  assert.match(rustSource, /start_native_watch/);
  assert.match(rustSource, /watch_generation/);
  assert.match(rustSource, /fallback_watch/);
  assert.doesNotMatch(rustSource, /std::mem::forget/);
  assert.match(appSource, /enter-lightweight-requested/);
  assert.match(windowSource, /enter_lightweight_mode/);
  assert.match(windowSource, /stop_native_watch_command/);
  assert.match(windowSource, /request_app_exit_command/);
  assert.match(watchSource, /stopNativeWatch\(\)/);

  // macOS: Dock policy + BSD-friendly orphan watch cleanup.
  assert.match(rustSource, /fn apply_tray_policy/);
  assert.match(rustSource, /ActivationPolicy::Accessory/);
  assert.match(rustSource, /ai-reminder\.js watch/);
  assert.match(lightweightSource, /\.visible\(true\)/);
  assert.match(lightweightSource, /MODE_LOCK/);
  assert.match(lightweightSource, /LIGHTWEIGHT_MODE\.store\(false/);

  // CloseRequested destroy skip is only the short-lived ALLOW flag.
  assert.match(lightweightSource, /fn allow_window_destroy/);
  assert.doesNotMatch(
    lightweightSource,
    /ALLOW_WINDOW_DESTROY\.load\(Ordering::Acquire\) \|\| is_lightweight_mode\(\)/,
  );

  // UI quit shares Rust cleanup path.
  assert.match(appSource, /requestAppExit/);

  // Listener must not depend on the whole watch object (re-subscribe every log line).
  assert.match(appSource, /\[watchRunning, watchStop\]/);

  // lightweightStart must not re-enter from the frontend after restore.
  assert.doesNotMatch(appSource, /if \(cfg\?\.ui\?\.lightweightStart\) \{\s*try \{\s*await enterLightweightMode/);
});

test('lightweightStart boots into tray-only mode without loading UI', () => {
  const rustSource = fs.readFileSync(path.join(root, 'src-tauri', 'src', 'lib.rs'), 'utf8');
  const defaultConfig = fs.readFileSync(path.join(root, 'src', 'default-config.js'), 'utf8');
  const advanced = fs.readFileSync(path.join(root, 'src-ui', 'components', 'AdvancedPanel.tsx'), 'utf8');
  const types = fs.readFileSync(path.join(root, 'src-ui', 'lib', 'types.ts'), 'utf8');
  const zh = fs.readFileSync(path.join(root, 'src-ui', 'i18n', 'zh-CN.json'), 'utf8');
  const en = fs.readFileSync(path.join(root, 'src-ui', 'i18n', 'en.json'), 'utf8');

  assert.match(defaultConfig, /lightweightStart:\s*false/);
  assert.match(types, /lightweightStart:\s*boolean/);
  assert.match(advanced, /lightweightStart/);
  assert.match(zh, /"advanced\.lightweightStart"/);
  assert.match(en, /"advanced\.lightweightStart"/);
  assert.match(rustSource, /fn read_lightweight_start_setting/);
  assert.match(rustSource, /lightweight_start/);
  assert.match(rustSource, /if lightweight_start \{[\s\S]*enter_lightweight_mode/);
});
