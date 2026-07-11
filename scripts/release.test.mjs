import { spawnSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import assert from 'node:assert/strict';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const scriptPath = resolve(repoRoot, 'scripts/release.mjs');
const packageVersion = JSON.parse(
  readFileSync(resolve(repoRoot, 'apps/desktop/package.json'), 'utf8'),
).version;
const workspaceVersion = readFileSync(resolve(repoRoot, 'Cargo.toml'), 'utf8').match(
  /^version = "([^"]+)"/m,
)?.[1];
const tauriVersion = JSON.parse(
  readFileSync(resolve(repoRoot, 'apps/desktop/src-tauri/tauri.conf.json'), 'utf8'),
).version;

test('workspace desktop and tauri versions match', () => {
  assert.equal(workspaceVersion, packageVersion);
  assert.equal(tauriVersion, packageVersion);
});

function runRelease(args) {
  return spawnSync(process.execPath, [scriptPath, ...args], {
    cwd: repoRoot,
    encoding: 'utf8',
  });
}

test('prints app release help', () => {
  const result = runRelease(['--help']);

  assert.equal(result.status, 0);
  assert.match(result.stdout, /Usage: node scripts\/release\.mjs/);
  assert.match(result.stdout, /desktop installers and Linux headless deb\/rpm artifacts/);
});

test('dry run prints app release command without pushing', () => {
  const result = runRelease(['--', '--dry-run', '--skip-fetch']);

  assert.equal(result.status, 0);
  assert.match(result.stdout, new RegExp(`Current app version: ${escapeRegExp(packageVersion)}`));
  assert.match(result.stdout, new RegExp(`Release tag: printbridge-v${escapeRegExp(packageVersion)}`));
  assert.match(result.stdout, /Command: git push origin HEAD:release/);
  assert.match(result.stdout, /headless \.deb and \.rpm/);
  assert.match(result.stdout, /Dry run only/);
});

function escapeRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}
