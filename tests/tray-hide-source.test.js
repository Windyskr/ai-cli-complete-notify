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

  assert.doesNotMatch(rustSource, /app\.hide\(\)/);
  assert.doesNotMatch(rustSource, /button_state:\s*tauri::tray::MouseButtonState::Up/);
  assert.match(rustSource, /RunEvent::Reopen/);
  assert.match(rustSource, /exit_lightweight_mode_impl\(app\)|restore_main_window\(app\)/);
});

test('tray lightweight mode destroys webview and keeps native watch', () => {
  const appSource = fs.readFileSync(path.join(root, 'src-ui', 'App.tsx'), 'utf8');
  const windowSource = fs.readFileSync(path.join(root, 'src-ui', 'lib', 'window.ts'), 'utf8');
  const watchSource = fs.readFileSync(path.join(root, 'src-ui', 'hooks', 'useWatch.ts'), 'utf8');
  const rustSource = fs.readFileSync(path.join(root, 'src-tauri', 'src', 'lib.rs'), 'utf8');

  assert.match(rustSource, /"lightweight"/);
  assert.match(rustSource, /fn enter_lightweight_mode_impl/);
  assert.match(rustSource, /destroy_main_window/);
  assert.match(rustSource, /start_native_watch/);
  assert.match(rustSource, /enter-lightweight-requested/);
  assert.match(rustSource, /WebviewWindowBuilder::new/);

  assert.match(appSource, /enter-lightweight-requested/);
  assert.match(appSource, /enterLightweightMode\(\)/);
  assert.match(windowSource, /enter_lightweight_mode/);
  assert.match(windowSource, /stop_native_watch_command/);
  assert.match(watchSource, /stopNativeWatch\(\)/);
});
