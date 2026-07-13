const os = require('os');
const path = require('path');

const PRODUCT_NAME = 'ai-cli-complete-notify';
const DATA_DIR_ENV = [
  'AI_CLI_COMPLETE_NOTIFY_DATA_DIR',
  'AICLI_COMPLETE_NOTIFY_DATA_DIR',
  'TASKPULSE_DATA_DIR',
  'AI_REMINDER_DATA_DIR'
];

function pickFirstEnv(names) {
  for (const name of names) {
    const value = process.env[name];
    if (typeof value === 'string' && value.trim()) return value.trim();
  }
  return '';
}

function getDataDir() {
  const override = pickFirstEnv(DATA_DIR_ENV);
  if (override) return path.resolve(override);

  const appData = process.env.APPDATA;
  if (appData) return path.join(appData, PRODUCT_NAME);

  return path.join(os.homedir(), `.${PRODUCT_NAME.toLowerCase()}`);
}

function getSettingsPath() {
  return path.join(getDataDir(), 'settings.json');
}

function getStatePath() {
  return path.join(getDataDir(), 'state.json');
}

function getWatchLogsDir() {
  return path.join(getDataDir(), 'watch-logs');
}

function formatWatchLogDate(date = new Date()) {
  const year = date.getFullYear();
  const month = String(date.getMonth() + 1).padStart(2, '0');
  const day = String(date.getDate()).padStart(2, '0');
  return `${year}-${month}-${day}`;
}

function getWatchLogPath(date = new Date()) {
  return path.join(getWatchLogsDir(), `watch-${formatWatchLogDate(date)}.log`);
}

function getLatestWatchLogPath() {
  const logDir = getWatchLogsDir();
  let fs;
  try {
    fs = require('fs');
  } catch (_error) {
    return '';
  }

  try {
    if (!fs.existsSync(logDir)) return '';
    const files = fs
      .readdirSync(logDir, { withFileTypes: true })
      .filter((entry) => entry && entry.isFile() && /^watch-\d{4}-\d{2}-\d{2}\.log$/i.test(entry.name))
      .map((entry) => entry.name)
      .sort((a, b) => b.localeCompare(a));
    if (files.length === 0) return '';
    return path.join(logDir, files[0]);
  } catch (_error) {
    return '';
  }
}

function getPrimaryEnvPath() {
  return path.join(getDataDir(), '.env');
}

function getExecutableEnvPath() {
  try {
    return path.join(path.dirname(process.execPath), '.env');
  } catch (_error) {
    return '';
  }
}

function isPackagedRuntime() {
  if (process.pkg) return true;
  if (String(process.env.AI_CLI_COMPLETE_NOTIFY_PACKAGED || '') === '1') return true;
  if (process.platform === 'darwin' && String(process.execPath || '').includes('.app/Contents/Resources/')) {
    return true;
  }
  return false;
}

function getEnvPathCandidates() {
  const candidates = [];
  const dataEnvPath = getPrimaryEnvPath();
  const executableEnvPath = getExecutableEnvPath();
  const cwdEnvPath = path.join(process.cwd(), '.env');

  // Packaged apps (Windows portable / macOS .app) keep settings.json and .env
  // under the same dataDir so UI "open data folder" matches what core loads.
  // Still accept a legacy .env next to the executable / cwd as fallback.
  if (isPackagedRuntime()) {
    candidates.push(dataEnvPath);
    if (executableEnvPath) candidates.push(executableEnvPath);
    candidates.push(cwdEnvPath);
    return [...new Set(candidates)];
  }

  // Dev / source: prefer cwd for local .env, then dataDir, then next-to-script.
  candidates.push(cwdEnvPath);
  candidates.push(dataEnvPath);
  if (executableEnvPath) candidates.push(executableEnvPath);

  return [...new Set(candidates)];
}

module.exports = {
  PRODUCT_NAME,
  getDataDir,
  getPrimaryEnvPath,
  getSettingsPath,
  getStatePath,
  getWatchLogsDir,
  getWatchLogPath,
  getLatestWatchLogPath,
  getEnvPathCandidates
};
