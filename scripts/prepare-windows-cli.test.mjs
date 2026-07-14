import assert from 'node:assert/strict';
import test from 'node:test';
import { mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { basename, dirname, resolve } from 'node:path';
import { tmpdir } from 'node:os';
import {
  prepareWindowsCli,
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

function temporaryRepository() {
  return mkdtempSync(resolve(tmpdir(), 'print-bridge-windows-cli-'));
}

function sidecarSourcePath(repositoryRoot, target) {
  return resolve(repositoryRoot, 'target', target, 'release', 'print-bridge-desktop-cli.exe');
}

test('failed Cargo build preserves an existing sidecar', () => {
  const repositoryRoot = temporaryRepository();
  const target = 'x86_64-pc-windows-msvc';
  const destination = sidecarOutputPath(repositoryRoot, target);
  mkdirSync(dirname(destination), { recursive: true });
  writeFileSync(destination, 'working sidecar');

  try {
    const exitCode = prepareWindowsCli(target, repositoryRoot, () => ({ status: 1 }));

    assert.equal(exitCode, 1);
    assert.equal(readFileSync(destination, 'utf8'), 'working sidecar');
  } finally {
    rmSync(repositoryRoot, { recursive: true, force: true });
  }
});

test('failed sidecar copy preserves an existing sidecar', () => {
  const repositoryRoot = temporaryRepository();
  const target = 'x86_64-pc-windows-msvc';
  const destination = sidecarOutputPath(repositoryRoot, target);
  mkdirSync(dirname(destination), { recursive: true });
  writeFileSync(destination, 'working sidecar');

  try {
    assert.throws(() => prepareWindowsCli(target, repositoryRoot, () => ({ status: 0 })));

    assert.equal(readFileSync(destination, 'utf8'), 'working sidecar');
  } finally {
    rmSync(repositoryRoot, { recursive: true, force: true });
  }
});

test('successful build replaces the sidecar and removes its temporary output', () => {
  const repositoryRoot = temporaryRepository();
  const target = 'x86_64-pc-windows-msvc';
  const destination = sidecarOutputPath(repositoryRoot, target);
  const source = sidecarSourcePath(repositoryRoot, target);
  mkdirSync(dirname(destination), { recursive: true });
  mkdirSync(dirname(source), { recursive: true });
  writeFileSync(destination, 'old sidecar');
  writeFileSync(source, 'new sidecar');

  try {
    const exitCode = prepareWindowsCli(target, repositoryRoot, () => ({ status: 0 }));

    assert.equal(exitCode, 0);
    assert.equal(readFileSync(destination, 'utf8'), 'new sidecar');
    assert.equal(
      readdirSync(dirname(destination)).some((entry) => entry.startsWith(`${basename(destination)}.`)),
      false,
    );
  } finally {
    rmSync(repositoryRoot, { recursive: true, force: true });
  }
});
