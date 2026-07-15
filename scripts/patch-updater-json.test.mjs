import test from 'node:test';
import assert from 'node:assert/strict';

import {
  findReleaseByTag,
  rewriteUpdaterAssetUrls,
  rewriteUpdaterReleaseNotes,
} from './patch-updater-json.mjs';

test('finds draft releases returned by the releases list endpoint', () => {
  const release = findReleaseByTag(
    [
      [{ tag_name: 'printbridge-v0.2.0', draft: false }],
      [{ tag_name: 'printbridge-v0.2.1', draft: true, assets: [] }],
    ],
    'printbridge-v0.2.1',
  );

  assert.equal(release.draft, true);
});

test('throws when the release list does not contain the requested tag', () => {
  assert.throws(
    () => findReleaseByTag([[{ tag_name: 'printbridge-v0.2.0' }]], 'printbridge-v0.2.1'),
    /Could not find release/,
  );
});

test('rewrites GitHub API asset URLs to browser download URLs', () => {
  const result = rewriteUpdaterAssetUrls(
    {
      version: '0.1.2',
      platforms: {
        'darwin-aarch64': {
          signature: 'sig',
          url: 'https://api.github.com/repos/vergil-lai/print-bridge/releases/assets/469097131',
        },
      },
    },
    {
      assets: [
        {
          id: 469097131,
          name: 'PrintBridge_0.1.2_aarch64.app.tar.gz',
          browser_download_url:
            'https://github.com/vergil-lai/print-bridge/releases/download/printbridge-v0.1.2/PrintBridge_0.1.2_aarch64.app.tar.gz',
        },
      ],
    },
  );

  assert.equal(
    result.platforms['darwin-aarch64'].url,
    'https://github.com/vergil-lai/print-bridge/releases/download/printbridge-v0.1.2/PrintBridge_0.1.2_aarch64.app.tar.gz',
  );
});

test('copies the GitHub release body into updater notes', () => {
  const result = rewriteUpdaterReleaseNotes(
    {
      version: '0.2.0',
      notes: 'placeholder',
      platforms: {},
    },
    '## PrintBridge v0.2.0\n\n- Added Linux headless packages.',
  );

  assert.equal(
    result.notes,
    '## PrintBridge v0.2.0\n\n- Added Linux headless packages.',
  );
});
