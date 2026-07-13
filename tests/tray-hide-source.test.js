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
  assert.match(rustSource, /allow_window_destroy/);
  assert.match(rustSource, /if allow_destroy \{\s*return;/);
  assert.match(rustSource, /fn request_enter_lightweight_mode/);
  assert.match(rustSource, /enter_lightweight_mode_impl\(app\)/);

  assert.match(appSource, /enter-lightweight-requested/);
  // Boot path may still call enterLightweightMode as a safety net.
  assert.match(windowSource, /enter_lightweight_mode/);
  assert.match(windowSource, /stop_native_watch_command/);
  assert.match(watchSource, /stopNativeWatch\(\)/);
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
  assert.match(rustSource, /if lightweight_start \{[\s\S]*enter_lightweight_mode_impl/);
});
