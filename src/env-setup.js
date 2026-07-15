const fs = require('fs');
const path = require('path');
const { getDataDir, getPrimaryEnvPath, getEnvPathCandidates } = require('./paths');

const ENV_PATH_ENV = [
  'AI_CLI_COMPLETE_NOTIFY_ENV_PATH',
  'AICLI_COMPLETE_NOTIFY_ENV_PATH',
  'TASKPULSE_ENV_PATH',
  'AI_REMINDER_ENV_PATH'
];

const FALLBACK_ENV_EXAMPLE = `# ai-cli-complete-notify environment config
# Copy this file to .env and fill in the notification settings you need.

NOTIFICATION_ENABLED=true
SOUND_ENABLED=true

# Webhook URLs for Feishu / DingTalk / WeCom. Separate multiple URLs with commas.
WEBHOOK_URLS=

# Telegram
TELEGRAM_BOT_TOKEN=
TELEGRAM_CHAT_ID=

# Gotify (self-hosted push notifications)
# GOTIFY_URL=https://gotify.example.com
# GOTIFY_APP_TOKEN=your_app_token

# Email
# EMAIL_HOST=smtp.example.com
# EMAIL_PORT=465
# EMAIL_SECURE=true
# EMAIL_USER=bot@example.com
# EMAIL_PASS=your_smtp_password
# EMAIL_FROM=AI Notify <bot@example.com>
# EMAIL_TO=your@email.com

# AI summary
# SUMMARY_ENABLED=false
# SUMMARY_API_URL=https://api.openai.com
# SUMMARY_API_KEY=
# SUMMARY_MODEL=gpt-4o-mini
`;

const WINDOWS_PATH_EXAMPLE = [
  '# 可选：数据目录/环境文件路径（便于 EXE 版固定存放位置）',
  '# AI_CLI_COMPLETE_NOTIFY_DATA_DIR=C:\\Users\\YourName\\AppData\\Roaming\\ai-cli-complete-notify',
  '# AI_CLI_COMPLETE_NOTIFY_ENV_PATH=C:\\Users\\YourName\\AppData\\Roaming\\ai-cli-complete-notify\\.env'
].join('\n');

const MACOS_PATH_EXAMPLE = [
  '# 可选：数据目录/环境文件路径（macOS 打包版默认使用下面这个目录）',
  '# AI_CLI_COMPLETE_NOTIFY_DATA_DIR=/Users/yourname/.ai-cli-complete-notify',
  '# AI_CLI_COMPLETE_NOTIFY_ENV_PATH=/Users/yourname/.ai-cli-complete-notify/.env'
].join('\n');

function pickFirstEnv(names) {
  for (const name of names) {
    const value = process.env[name];
    if (typeof value === 'string' && value.trim()) return value.trim();
  }
  return '';
}

function exists(filePath) {
  try {
    return Boolean(filePath) && fs.existsSync(filePath);
  } catch (_error) {
    return false;
  }
}

function uniquePaths(paths) {
  return [...new Set(paths.filter(Boolean))];
}

function isPackagedRuntime() {
  // Keep in sync with paths.js — macOS .app layout must count as packaged so
  // recommendedEnvPath / dataDir match what core loads.
  if (process.pkg) return true;
  if (String(process.env.AI_CLI_COMPLETE_NOTIFY_PACKAGED || '') === '1') return true;
  if (process.platform === 'darwin' && String(process.execPath || '').includes('.app/Contents/Resources/')) {
    return true;
  }
  return false;
}

function formatTemplateForRuntime(content) {
  if (process.platform === 'darwin' && isPackagedRuntime()) {
    return content.replace(WINDOWS_PATH_EXAMPLE, MACOS_PATH_EXAMPLE);
  }
  return content;
}

function getTemplateContent() {
  const candidates = [
    path.join(__dirname, '..', '.env.example'),
    path.join(process.cwd(), '.env.example')
  ];

  for (const candidate of candidates) {
    try {
      if (fs.existsSync(candidate)) return formatTemplateForRuntime(fs.readFileSync(candidate, 'utf8'));
    } catch (_error) {
      // ignore and try fallback
    }
  }

  return formatTemplateForRuntime(FALLBACK_ENV_EXAMPLE);
}

function getRecommendedEnvPath(explicitPath) {
  // Recommended write path must match the first candidate core would load when
  // no legacy file exists. Packaged + dataDir keeps settings.json and .env together.
  if (explicitPath) return explicitPath;
  if (isPackagedRuntime()) return getPrimaryEnvPath();
  return path.join(process.cwd(), '.env');
}

function getEnvSetupStatus(options = {}) {
  const createExample = Boolean(options.createExample);
  const explicit = pickFirstEnv(ENV_PATH_ENV);
  const explicitPath = explicit ? path.resolve(explicit) : '';
  // Same search order as bootstrapEnv(): explicit override → shared candidates.
  const envCandidates = uniquePaths([
    explicitPath,
    ...getEnvPathCandidates(),
    path.join(__dirname, '..', '.env')
  ]);
  const recommendedEnvPath = getRecommendedEnvPath(explicitPath);
  // What core actually loaded (first existing file in bootstrap order).
  const loadedEnvPath = envCandidates.find((candidate) => exists(candidate)) || '';
  const envExists = Boolean(loadedEnvPath);
  // UI should open the folder that matches the truth: loaded file if any, else
  // the recommended write location next to settings.
  const displayEnvPath = loadedEnvPath || recommendedEnvPath;
  const examplePath = path.join(path.dirname(recommendedEnvPath), '.env.example');
  let exampleCreated = false;
  let exampleExists = exists(examplePath);
  let error = '';

  if (!envExists && createExample && !exampleExists) {
    try {
      fs.mkdirSync(path.dirname(examplePath), { recursive: true });
      fs.writeFileSync(examplePath, getTemplateContent(), 'utf8');
      exampleCreated = true;
      exampleExists = true;
    } catch (err) {
      error = err && err.message ? err.message : String(err);
    }
  }

  return {
    ok: !error,
    status: envExists ? 'loaded' : 'missing',
    dataDir: getDataDir(),
    // Canonical fields for UI:
    // - envPath: path to show / recommend (loaded if present, else write target)
    // - loadedEnvPath: actual file core loaded (empty when missing)
    // - recommendedEnvPath: where user should create .env when missing
    envPath: displayEnvPath,
    loadedEnvPath,
    recommendedEnvPath,
    envCandidates,
    envExists,
    examplePath,
    exampleExists,
    exampleCreated,
    error
  };
}

module.exports = {
  formatTemplateForRuntime,
  getEnvSetupStatus
};
