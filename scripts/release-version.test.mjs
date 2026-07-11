import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

import { isPrerelease, toLinuxPackageVersion } from './release-version.mjs';

test('stable versions remain stable Linux package versions', () => {
  assert.equal(toLinuxPackageVersion('0.2.0'), '0.2.0');
  assert.equal(isPrerelease('0.2.0'), false);
});

test('SemVer prerelease versions sort before the final Linux package version', () => {
  assert.equal(toLinuxPackageVersion('0.2.0-dev.1'), '0.2.0~dev.1');
  assert.equal(isPrerelease('0.2.0-dev.1'), true);
});

test('release workflow marks SemVer prereleases as GitHub prereleases', () => {
  const workflow = readFileSync('.github/workflows/release.yml', 'utf8');

  assert.match(workflow, /release-version\.mjs prerelease/);
  assert.match(workflow, /prerelease: \$\{\{ steps\.release_version\.outputs\.prerelease \}\}/);
});

test('headless packaging uses the normalized Linux package version', () => {
  const script = readFileSync('scripts/build-server-packages.sh', 'utf8');

  assert.match(script, /release-version\.mjs" linux/);
  assert.ok(script.includes('s/\\${VERSION}/$PACKAGE_VERSION/'));
  assert.ok(script.includes('s/__VERSION__/$PACKAGE_VERSION/'));
});
