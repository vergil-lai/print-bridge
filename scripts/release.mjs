import { spawnSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { createInterface } from 'node:readline/promises';
import { stdin as input, stdout as output } from 'node:process';

const REPO = 'vergil-lai/print-bridge';
const WORKFLOW = 'release.yml';
const TAG_PREFIX = 'printbridge-v';

const args = process.argv.slice(2);
let dryRun = false;
let yes = false;
let skipFetch = false;
let target = null;

for (const arg of args) {
  switch (arg) {
    case '--':
      break;
    case 'app':
    case 'desktop':
    case 'printbridge':
      target = 'app';
      break;
    case '--dry-run':
      dryRun = true;
      break;
    case '-y':
    case '--yes':
      yes = true;
      break;
    case '--skip-fetch':
      skipFetch = true;
      break;
    case '-h':
    case '--help':
      printHelp();
      process.exit(0);
      break;
    default:
      fail(`Unexpected argument: ${arg}`);
  }
}

if (target && target !== 'app') {
  fail(`Unknown release target: ${target}`);
}

const repoRoot = run('git', ['rev-parse', '--show-toplevel']).stdout.trim();
process.chdir(repoRoot);

if (!skipFetch) {
  fetchReleaseTags();
}

await releaseApp();

async function releaseApp() {
  const versions = readAppVersions();
  const versionSet = new Set(Object.values(versions));
  if (versionSet.size !== 1) {
    fail(
      `Version fields differ:\n` +
        `  apps/desktop/package.json: ${versions.packageJson}\n` +
        `  apps/desktop/src-tauri/tauri.conf.json: ${versions.tauriConf}\n` +
        `  Cargo.toml workspace package: ${versions.cargoToml}`,
    );
  }

  const version = versions.packageJson;
  const releaseTag = `${TAG_PREFIX}${version}`;

  if (tagExists(releaseTag)) {
    fail(`Tag ${releaseTag} already exists.`);
  }

  console.log('Release target: PrintBridge desktop + headless server');
  console.log('Artifacts: desktop installers, headless .deb and .rpm');
  console.log(`Repository: ${REPO}`);
  console.log(`Current app version: ${version}`);
  console.log(`Release tag: ${releaseTag}`);
  console.log(`Workflow: ${WORKFLOW}`);
  console.log('Command: git push origin HEAD:release');

  if (dryRun) {
    console.log('Dry run only; release branch was not pushed.');
    return;
  }

  ensureCleanWorktree();
  await confirmOrExit(`Confirm pushing HEAD to origin/release for ${releaseTag}? [y/N] `);

  run('git', ['push', 'origin', 'HEAD:release'], { stdio: 'inherit' });
  console.log(`Pushed HEAD to origin/release; ${WORKFLOW} will create ${releaseTag}.`);
}

function readAppVersions() {
  const packageJson = JSON.parse(readFileSync('apps/desktop/package.json', 'utf8')).version;
  const tauriConf = JSON.parse(
    readFileSync('apps/desktop/src-tauri/tauri.conf.json', 'utf8'),
  ).version;
  const cargoToml = readFileSync('Cargo.toml', 'utf8').match(
    /^version = "([^"]+)"/m,
  )?.[1];

  if (!packageJson || !tauriConf || !cargoToml) {
    fail('Could not read version from package.json, tauri.conf.json, or workspace Cargo.toml.');
  }

  return { packageJson, tauriConf, cargoToml };
}

async function confirmOrExit(message) {
  if (yes) return;
  if (!process.stdin.isTTY) {
    fail('Refusing to trigger release without confirmation in a non-interactive shell. Re-run with --yes if this is intentional.');
  }

  const rl = createInterface({ input, output });
  const answer = await rl.question(message);
  rl.close();

  if (!['y', 'yes'].includes(answer.trim().toLowerCase())) {
    console.log('Cancelled.');
    process.exit(0);
  }
}

function ensureCleanWorktree() {
  const status = run('git', ['status', '--porcelain']).stdout.trim();
  if (status) {
    fail(`Working tree is not clean. Commit or stash changes before releasing:\n${status}`);
  }
}

function fetchReleaseTags() {
  run('git', ['fetch', '--quiet', 'origin', `+refs/tags/${TAG_PREFIX}*:refs/tags/${TAG_PREFIX}*`]);
}

function tagExists(tag) {
  const result = spawnSync('git', ['rev-parse', '-q', '--verify', `refs/tags/${tag}`], {
    cwd: process.cwd(),
    encoding: 'utf8',
    stdio: 'pipe',
  });
  return result.status === 0;
}

function run(command, commandArgs, options = {}) {
  const result = spawnSync(command, commandArgs, {
    cwd: process.cwd(),
    env: process.env,
    encoding: 'utf8',
    stdio: options.stdio ?? 'pipe',
  });

  if (result.error) {
    fail(`${command} ${commandArgs.join(' ')} failed: ${result.error.message}`);
  }

  if (result.status !== 0) {
    const stderr = typeof result.stderr === 'string' ? result.stderr.trim() : '';
    fail(`${command} ${commandArgs.join(' ')} failed${stderr ? `:\n${stderr}` : ''}`);
  }

  return result;
}

function fail(message) {
  console.error(message);
  process.exit(1);
}

function printHelp() {
  console.log(`Usage: node scripts/release.mjs [app] [options]

Release target: PrintBridge desktop installers and Linux headless deb/rpm artifacts.

This script validates that apps/desktop/package.json,
apps/desktop/src-tauri/tauri.conf.json, and apps/desktop/src-tauri/Cargo.toml
use the same version, then pushes the current commit to
origin/release. The existing GitHub Actions workflow creates the release tag
with the printbridge-vX.Y.Z format.

Options:
  --dry-run             Print the release command without pushing
  -y, --yes             Skip the confirmation prompt
  --skip-fetch          Do not fetch release tags before checking
  -h, --help            Show this help
`);
}
