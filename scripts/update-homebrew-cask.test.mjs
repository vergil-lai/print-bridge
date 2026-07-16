import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';

import { findReleaseDmgs, rewriteHomebrewCask, sha256File } from './update-homebrew-cask.mjs';

const cask = `cask "printbridge" do
  arch arm: "aarch64", intel: "x64"

  version "0.1.3"
  sha256 arm:   "old-arm-sha",
         intel: "old-intel-sha"

  url "https://github.com/vergil-lai/print-bridge/PrintBridge_#{version}_#{arch}.dmg",
      verified: "github.com/vergil-lai/print-bridge/"
end
`;

test('updates the version and both architecture checksums', () => {
  const result = rewriteHomebrewCask(cask, {
    version: '0.2.0',
    armSha256: 'new-arm-sha',
    intelSha256: 'new-intel-sha',
  });

  assert.match(result, /version "0\.2\.0"/);
  assert.match(result, /sha256 arm:\s+"new-arm-sha",\n\s+intel: "new-intel-sha"/);
  assert.match(result, /verified: "github\.com\/vergil-lai\/print-bridge\/"/);
  assert.doesNotMatch(result, /old-(arm|intel)-sha/);
});

test('rejects a cask without the expected dual-architecture checksum stanza', () => {
  assert.throws(
    () =>
      rewriteHomebrewCask('cask "printbridge" do\n  version "0.1.3"\nend\n', {
        version: '0.2.0',
        armSha256: 'arm-sha',
        intelSha256: 'intel-sha',
      }),
    /dual-architecture sha256 stanza/,
  );
});

test('finds exactly one Apple Silicon and one Intel DMG', () => {
  const directory = mkdtempSync(join(tmpdir(), 'printbridge-homebrew-assets-'));
  const armPath = join(directory, 'PrintBridge_0.2.0_aarch64.dmg');
  const intelPath = join(directory, 'PrintBridge_0.2.0_x64.dmg');
  writeFileSync(armPath, 'arm');
  writeFileSync(intelPath, 'intel');

  assert.deepEqual(findReleaseDmgs(directory), { armPath, intelPath });
});

test('rejects missing or ambiguous DMG assets', () => {
  const directory = mkdtempSync(join(tmpdir(), 'printbridge-homebrew-assets-'));
  writeFileSync(join(directory, 'PrintBridge_0.2.0_aarch64.dmg'), 'arm');
  writeFileSync(join(directory, 'PrintBridge_0.2.0_arm64.dmg'), 'arm-duplicate');

  assert.throws(() => findReleaseDmgs(directory), /exactly one Apple Silicon DMG/);
});

test('calculates a file SHA256 checksum', async () => {
  const directory = mkdtempSync(join(tmpdir(), 'printbridge-homebrew-assets-'));
  const filePath = join(directory, 'asset.dmg');
  writeFileSync(filePath, 'printbridge');

  assert.equal(
    await sha256File(filePath),
    createHash('sha256').update('printbridge').digest('hex'),
  );
});

test('Homebrew workflow updates the tap only after a stable release is published', () => {
  const workflow = readFileSync('.github/workflows/update-homebrew.yml', 'utf8');

  assert.match(workflow, /release:\n\s+types:\n\s+- published/);
  assert.match(workflow, /workflow_dispatch:/);
  assert.match(workflow, /tag:\n\s+description: GitHub release tag/);
  assert.match(
    workflow,
    /if: \$\{\{ github\.event_name == 'workflow_dispatch' \|\| !github\.event\.release\.prerelease \}\}/,
  );
  assert.match(workflow, /secrets\.HOMEBREW_TAP_TOKEN/);
  assert.match(workflow, /update-homebrew-cask\.mjs/);
  assert.match(workflow, /brew style --fix Casks\/printbridge\.rb/);
  assert.match(workflow, /brew tap "\$\{TAP_NAME\}"/);
  assert.match(workflow, /TAP_PATH="\$\(brew --repo "\$\{TAP_NAME\}"\)"/);
  assert.match(workflow, /cp Casks\/printbridge\.rb "\$\{TAP_PATH\}\/Casks\/printbridge\.rb"/);
  assert.match(workflow, /brew trust --cask "\$\{TAP_NAME\}\/printbridge"/);
  assert.match(workflow, /brew audit --cask --strict "\$\{TAP_NAME\}\/printbridge"/);
  assert.doesNotMatch(workflow, /brew audit --cask --strict tap\/Casks\/printbridge\.rb/);
  assert.match(workflow, /git -C tap push origin HEAD:main/);
  assert.doesNotMatch(workflow, /automation\/printbridge-/);
  assert.doesNotMatch(workflow, /gh pr (?:list|create)/);
});
