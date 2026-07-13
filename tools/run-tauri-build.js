const fs = require('fs');
const os = require('os');
const path = require('path');
const { spawnSync } = require('child_process');

function prependPath(env, value) {
  const pathKey = Object.keys(env).find((key) => key.toLowerCase() === 'path') || 'Path';
  const current = env[pathKey] || '';
  env[pathKey] = `${value}${path.delimiter}${current}`;
}

function ensureDir(dirPath) {
  fs.mkdirSync(dirPath, { recursive: true });
}

function ensureWritableDir(candidates) {
  const errors = [];
  for (const dirPath of candidates) {
    if (!dirPath) continue;
    try {
      ensureDir(dirPath);
      // Prefer a path we can actually write to (GitHub Actions may not have D:).
      const probe = path.join(dirPath, `.write-probe-${process.pid}`);
      fs.writeFileSync(probe, 'ok');
      fs.unlinkSync(probe);
      return dirPath;
    } catch (error) {
      errors.push(`${dirPath}: ${error && error.message ? error.message : error}`);
    }
  }
  throw new Error(`No writable temp/cache directory. Tried: ${errors.join(' | ')}`);
}

function resolveWindowsRustEnv(rootDir) {
  const env = { ...process.env };

  if (process.platform !== 'win32') {
    return env;
  }

  const candidateCargoHome = [
    env.CARGO_HOME,
    'D:\\cargo',
    path.join(process.env.USERPROFILE || '', '.cargo')
  ].filter(Boolean);
  const candidateRustupHome = [
    env.RUSTUP_HOME,
    'D:\\rustup',
    path.join(process.env.USERPROFILE || '', '.rustup')
  ].filter(Boolean);

  const cargoHome = candidateCargoHome.find((dir) =>
    fs.existsSync(path.join(dir, 'bin', 'cargo.exe'))
  );
  const rustupHome = candidateRustupHome.find((dir) => fs.existsSync(dir));

  if (!cargoHome) {
    throw new Error(
      'Rust toolchain not found. Install Rust or set CARGO_HOME so `cargo.exe` is available.'
    );
  }

  env.CARGO_HOME = cargoHome;
  if (rustupHome) {
    env.RUSTUP_HOME = rustupHome;
  }
  prependPath(env, path.dirname(process.execPath));
  prependPath(env, path.join(cargoHome, 'bin'));

  const tempDir = ensureWritableDir([
    env.AI_NOTIFY_TMP_DIR,
    process.env.RUNNER_TEMP && path.join(process.env.RUNNER_TEMP, 'ai-cli-complete-notify'),
    'D:\\tmp',
    path.join(os.tmpdir(), 'ai-cli-complete-notify'),
  ]);
  env.TEMP = tempDir;
  env.TMP = tempDir;

  const npmCacheDir = ensureWritableDir([
    env.npm_config_cache,
    process.env.RUNNER_TEMP && path.join(process.env.RUNNER_TEMP, 'npm-cache'),
    'D:\\npm-cache',
    path.join(os.tmpdir(), 'ai-cli-complete-notify-npm-cache'),
  ]);
  env.npm_config_cache = npmCacheDir;

  return env;
}

function main() {
  const rootDir = path.join(__dirname, '..');
  const args = process.argv.slice(2);

  let env;
  try {
    env = resolveWindowsRustEnv(rootDir);
  } catch (error) {
    console.error(`[tauri-build] ${error.message}`);
    process.exit(1);
  }

  const tauriCliPath = path.join(rootDir, 'node_modules', '@tauri-apps', 'cli', 'tauri.js');
  if (!fs.existsSync(tauriCliPath)) {
    console.error('[tauri-build] Tauri CLI not found. Run `npm install` first.');
    process.exit(1);
  }

  const result = spawnSync(process.execPath, [tauriCliPath, 'build', ...args], {
    cwd: rootDir,
    env,
    stdio: 'inherit',
    shell: false
  });

  if (typeof result.status === 'number') {
    process.exit(result.status);
  }

  console.error('[tauri-build] Failed to start Tauri build process.');
  process.exit(1);
}

main();
