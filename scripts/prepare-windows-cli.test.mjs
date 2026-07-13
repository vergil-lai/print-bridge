import assert from 'node:assert/strict';
import test from 'node:test';
import { resolve } from 'node:path';
import { sidecarOutputPath, validateWindowsTarget } from './prepare-windows-cli.mjs';

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
