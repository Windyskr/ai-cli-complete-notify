const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

const { getEnvSetupStatus } = require('../src/env-setup');

const ENV_KEYS = [
  'APPDATA',
  'AI_CLI_COMPLETE_NOTIFY_DATA_DIR',
  'AICLI_COMPLETE_NOTIFY_DATA_DIR',
  'TASKPULSE_DATA_DIR',
  'AI_REMINDER_DATA_DIR',
  'AI_CLI_COMPLETE_NOTIFY_ENV_PATH',
  'AICLI_COMPLETE_NOTIFY_ENV_PATH',
  'TASKPULSE_ENV_PATH',
  'AI_REMINDER_ENV_PATH',
  'AI_CLI_COMPLETE_NOTIFY_PACKAGED'
];

function withTempHome(fn) {
  const tempHome = fs.mkdtempSync(path.join(os.tmpdir(), 'ai-notify-env-'));
  const saved = {
    HOME: process.env.HOME,
    USERPROFILE: process.env.USERPROFILE,
  };
  const savedEnv = {};
  for (const key of ENV_KEYS) savedEnv[key] = process.env[key];

  process.env.HOME = tempHome;
  process.env.USERPROFILE = tempHome;
  for (const key of ENV_KEYS) delete process.env[key];
  process.env.AI_CLI_COMPLETE_NOTIFY_PACKAGED = '1';

  try {
    return fn(tempHome);
  } finally {
    if (saved.HOME === undefined) delete process.env.HOME;
    else process.env.HOME = saved.HOME;
    if (saved.USERPROFILE === undefined) delete process.env.USERPROFILE;
    else process.env.USERPROFILE = saved.USERPROFILE;
    for (const key of ENV_KEYS) {
      if (savedEnv[key] === undefined) delete process.env[key];
      else process.env[key] = savedEnv[key];
    }
    fs.rmSync(tempHome, { recursive: true, force: true });
  }
}

function withTempCwd(fn) {
  const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), 'ai-notify-cwd-'));
  const savedCwd = process.cwd();
  const savedHome = {
    HOME: process.env.HOME,
    USERPROFILE: process.env.USERPROFILE,
  };
  const savedEnv = {};
  for (const key of ENV_KEYS) savedEnv[key] = process.env[key];

  for (const key of ENV_KEYS) delete process.env[key];
  process.env.HOME = tempDir;
  process.env.USERPROFILE = tempDir;
  process.chdir(tempDir);

  try {
    return fn(tempDir);
  } finally {
    process.chdir(savedCwd);
    if (savedHome.HOME === undefined) delete process.env.HOME;
    else process.env.HOME = savedHome.HOME;
    if (savedHome.USERPROFILE === undefined) delete process.env.USERPROFILE;
    else process.env.USERPROFILE = savedHome.USERPROFILE;
    for (const key of ENV_KEYS) {
      if (savedEnv[key] === undefined) delete process.env[key];
      else process.env[key] = savedEnv[key];
    }
    fs.rmSync(tempDir, { recursive: true, force: true });
  }
}

test('env setup creates .env.example in the data dir when packaged macOS env is missing', () => {
  if (process.platform !== 'darwin') return;

  withTempHome((tempHome) => {
    const expectedDir = path.join(tempHome, '.ai-cli-complete-notify');
    const expectedEnvPath = path.join(expectedDir, '.env');
    const expectedExamplePath = path.join(expectedDir, '.env.example');

    const status = getEnvSetupStatus({ createExample: true });

    assert.equal(status.status, 'missing');
    assert.equal(status.envExists, false);
    assert.equal(status.envPath, expectedEnvPath);
    assert.equal(status.examplePath, expectedExamplePath);
    assert.equal(status.exampleCreated, true);
    assert.equal(fs.existsSync(expectedExamplePath), true);
    const content = fs.readFileSync(expectedExamplePath, 'utf8');
    assert.match(content, /WEBHOOK_URLS=/);
    assert.match(content, /AI_CLI_COMPLETE_NOTIFY_ENV_PATH=\/Users\/yourname\/\.ai-cli-complete-notify\/\.env/);
    assert.equal(content.includes('C:\\Users'), false);
  });
});

test('env setup reports loaded when packaged macOS env exists', () => {
  withTempHome((tempHome) => {
    const dataDir = path.join(tempHome, '.ai-cli-complete-notify');
    const envPath = path.join(dataDir, '.env');
    fs.mkdirSync(dataDir, { recursive: true });
    fs.writeFileSync(envPath, 'WEBHOOK_URLS=https://example.test/hook\n', 'utf8');

    const status = getEnvSetupStatus({ createExample: true });

    assert.equal(status.status, 'loaded');
    assert.equal(status.envExists, true);
    assert.equal(status.envPath, envPath);
    assert.equal(status.loadedEnvPath, envPath);
    assert.equal(status.exampleCreated, false);
    assert.equal(fs.existsSync(path.join(dataDir, '.env.example')), false);
  });
});

test('env setup creates .env.example next to the source cwd in dev mode', () => {
  withTempCwd((tempDir) => {
    const status = getEnvSetupStatus({ createExample: true });
    const cwd = process.cwd();

    assert.equal(status.status, 'missing');
    assert.equal(status.envPath, path.join(cwd, '.env'));
    assert.equal(status.recommendedEnvPath, path.join(cwd, '.env'));
    assert.equal(status.examplePath, path.join(cwd, '.env.example'));
    assert.equal(status.exampleCreated, true);
    assert.equal(fs.existsSync(path.join(tempDir, '.env.example')), true);
  });
});

test('packaged runtime recommends dataDir .env and reports loaded path from core order', () => {
  withTempHome((tempHome) => {
    process.env.AI_CLI_COMPLETE_NOTIFY_PACKAGED = '1';
    process.env.AI_CLI_COMPLETE_NOTIFY_DATA_DIR = path.join(tempHome, 'data');
    // Simulate portable layout: legacy .env next to a fake execPath is not needed;
    // dataDir is the canonical write/load location.
    const dataDir = process.env.AI_CLI_COMPLETE_NOTIFY_DATA_DIR;
    const envPath = path.join(dataDir, '.env');
    fs.mkdirSync(dataDir, { recursive: true });

    const missing = getEnvSetupStatus({ createExample: true });
    assert.equal(missing.status, 'missing');
    assert.equal(missing.dataDir, dataDir);
    assert.equal(missing.recommendedEnvPath, envPath);
    assert.equal(missing.envPath, envPath);
    assert.equal(missing.loadedEnvPath, '');
    assert.equal(missing.examplePath, path.join(dataDir, '.env.example'));
    assert.equal(missing.exampleCreated, true);

    fs.writeFileSync(envPath, 'WEBHOOK_URLS=https://example.test/hook\n', 'utf8');
    const loaded = getEnvSetupStatus({ createExample: false });
    assert.equal(loaded.status, 'loaded');
    assert.equal(loaded.loadedEnvPath, envPath);
    assert.equal(loaded.envPath, envPath);
    assert.equal(loaded.recommendedEnvPath, envPath);
  });
});
