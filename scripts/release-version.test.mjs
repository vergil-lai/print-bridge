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

test('desktop and headless publishing are independent after release preparation', () => {
  const workflow = readFileSync('.github/workflows/release.yml', 'utf8');

  assert.match(workflow, /prepare-release:\n\s+needs: quality/);
  assert.match(workflow, /publish-tauri:\n\s+needs: prepare-release/);
  assert.match(workflow, /publish-headless:\n\s+needs: prepare-release/);
  assert.doesNotMatch(workflow, /publish-headless:\n\s+needs: publish-tauri/);
});

test('release workflow installs AppImage tools and separates NSIS from MSI', () => {
  const workflow = readFileSync('.github/workflows/release.yml', 'utf8');

  assert.match(workflow, /xdg-utils/);
  assert.match(workflow, /--bundles nsis\n/);
  assert.match(workflow, /--bundles msi\n/);
  assert.doesNotMatch(workflow, /--bundles nsis,msi/);
});

test('release workflow keeps the release as a draft until manual publication', () => {
  const workflow = readFileSync('.github/workflows/release.yml', 'utf8');

  assert.match(workflow, /gh release create/);
  assert.match(workflow, /ARGS=\([\s\S]*--draft/);
  assert.match(workflow, /releaseDraft: true/);
});

test('release note sync workflow updates updater notes after edits and publication', () => {
  const workflow = readFileSync('.github/workflows/sync-release-notes.yml', 'utf8');

  assert.match(workflow, /types: \[edited, published\]/);
  assert.match(workflow, /printbridge-v/);
  assert.match(workflow, /node scripts\/patch-updater-json\.mjs/);
});

test('headless packaging uses the normalized Linux package version', () => {
  const script = readFileSync('scripts/build-server-packages.sh', 'utf8');
  const controlTemplate = readFileSync('apps/server/packaging/deb/control', 'utf8');
  const renderedControl = controlTemplate
    .replace('${VERSION}', '0.2.0~dev.2')
    .replace('${ARCH}', 'amd64');

  assert.match(script, /release-version\.mjs" linux/);
  assert.ok(script.includes('s/\\${VERSION}/$PACKAGE_VERSION/'));
  assert.ok(script.includes('s/__VERSION__/$PACKAGE_VERSION/'));
  assert.match(renderedControl, /^Version: [0-9]/m);
  assert.match(renderedControl, /^Architecture: amd64$/m);
});
