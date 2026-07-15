import { spawnSync } from 'node:child_process';
import { copyFileSync, mkdirSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');

export function validateWindowsTarget(target) {
  if (!/^[^-]+-pc-windows-(?:msvc|gnu)$/.test(target)) {
    throw new Error(`Expected a Windows target triple, received: ${target}`);
  }
}

export function sidecarOutputPath(repositoryRoot, target) {
  return resolve(
    repositoryRoot,
    'apps/desktop/src-tauri/binaries',
    `print-bridge-${target}.exe`,
  );
}

export function sidecarCargoArgs(target) {
  return [
    'build',
    '--release',
    '-p',
    'print-bridge-desktop',
    '--bin',
    'print-bridge-desktop-cli',
    '--features',
    'windows-cli',
    '--target',
    target,
  ];
}

export function sidecarBuildEnvironment(environment = process.env) {
  return {
    ...environment,
    TAURI_CONFIG: JSON.stringify({ bundle: { externalBin: [] } }),
  };
}

export function prepareWindowsCli(target, repositoryRoot = root) {
  validateWindowsTarget(target);
  const result = spawnSync('cargo', sidecarCargoArgs(target), {
    cwd: repositoryRoot,
    env: sidecarBuildEnvironment(),
    stdio: 'inherit',
  });
  if (result.status !== 0) process.exit(result.status ?? 1);

  const source = resolve(repositoryRoot, 'target', target, 'release', 'print-bridge-desktop-cli.exe');
  const destination = sidecarOutputPath(repositoryRoot, target);
  mkdirSync(dirname(destination), { recursive: true });
  copyFileSync(source, destination);
  console.log(`Prepared Windows CLI sidecar: ${destination}`);
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  prepareWindowsCli(process.argv[2] ?? '');
}
