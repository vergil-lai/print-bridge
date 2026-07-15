import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import {
  sidecarBuildEnvironment,
  sidecarCargoArgs,
  sidecarOutputPath,
  validateWindowsTarget,
} from './prepare-windows-cli.mjs';

test('sidecar output includes the full Windows target triple', () => {
  assert.equal(
    sidecarOutputPath('/repo', 'x86_64-pc-windows-msvc'),
    resolve(
      '/repo',
      'apps/desktop/src-tauri/binaries/print-bridge-x86_64-pc-windows-msvc.exe',
    ),
  );
});

test('non-Windows targets are rejected', () => {
  assert.throws(
    () => validateWindowsTarget('aarch64-apple-darwin'),
    /Windows target triple/,
  );
});

test('sidecar build enables only the Windows CLI binary feature', () => {
  assert.deepEqual(sidecarCargoArgs('x86_64-pc-windows-msvc'), [
    'build',
    '--release',
    '-p',
    'print-bridge-desktop',
    '--bin',
    'print-bridge-desktop-cli',
    '--features',
    'windows-cli',
    '--target',
    'x86_64-pc-windows-msvc',
  ]);
});

test('sidecar build clears externalBin until the executable has been copied', () => {
  const environment = sidecarBuildEnvironment({ PATH: '/bin' });

  assert.equal(environment.PATH, '/bin');
  assert.deepEqual(JSON.parse(environment.TAURI_CONFIG), {
    bundle: { externalBin: [] },
  });
});

test('desktop CLI binary is gated behind the Windows-only build feature', () => {
  const manifest = readFileSync('apps/desktop/src-tauri/Cargo.toml', 'utf8');

  assert.match(manifest, /\[features\][\s\S]*windows-cli = \[\]/);
  assert.match(
    manifest,
    /name = "print-bridge-desktop-cli"[\s\S]*required-features = \["windows-cli"\]/,
  );
});
