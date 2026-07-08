import { spawnSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import assert from 'node:assert/strict';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const scriptPath = resolve(repoRoot, 'scripts/release.mjs');
const packageVersion = JSON.parse(readFileSync(resolve(repoRoot, 'package.json'), 'utf8')).version;

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
  assert.match(result.stdout, /Release target: PrintBridge app/);
});

test('dry run prints app release command without pushing', () => {
  const result = runRelease(['--', '--dry-run', '--skip-fetch']);

  assert.equal(result.status, 0);
  assert.match(result.stdout, new RegExp(`Current app version: ${escapeRegExp(packageVersion)}`));
  assert.match(result.stdout, new RegExp(`Release tag: printbridge-v${escapeRegExp(packageVersion)}`));
  assert.match(result.stdout, /Command: git push origin HEAD:release/);
  assert.match(result.stdout, /Dry run only/);
});

function escapeRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}
