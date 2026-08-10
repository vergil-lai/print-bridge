import test from 'node:test';
import assert from 'node:assert/strict';

import {
  downloadLatestJsonWithRetry,
  findReleaseByTag,
  rewriteUpdaterAssetUrls,
  rewriteUpdaterReleaseNotes,
} from './patch-updater-json.mjs';

test('retries downloading latest.json until the asset becomes available', async () => {
  let downloads = 0;
  const waits = [];

  await downloadLatestJsonWithRetry(
    () => {
      downloads += 1;
      if (downloads < 3) {
        throw new Error('gh release download failed:\nno assets match the file pattern');
      }
    },
    {
      attempts: 3,
      wait: async (milliseconds) => waits.push(milliseconds),
    },
  );

  assert.equal(downloads, 3);
  assert.deepEqual(waits, [3_000, 3_000]);
});

test('does not retry unrelated latest.json download failures', async () => {
  let downloads = 0;

  await assert.rejects(
    downloadLatestJsonWithRetry(
      () => {
        downloads += 1;
        throw new Error('gh release download failed:\nHTTP 403: Resource not accessible by integration');
      },
      {
        attempts: 3,
        wait: async () => assert.fail('unexpected retry'),
      },
    ),
    /HTTP 403/,
  );

  assert.equal(downloads, 1);
});

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

test('rewrites draft release download URLs to the published release asset URL', () => {
  const result = rewriteUpdaterAssetUrls(
    {
      version: '0.2.4',
      platforms: {
        'darwin-aarch64': {
          signature: 'sig',
          url: 'https://github.com/vergil-lai/print-bridge/releases/download/untagged-98f3539f98525d473150/PrintBridge_0.2.4_aarch64.app.tar.gz',
        },
      },
    },
    {
      assets: [
        {
          id: 506256526,
          name: 'PrintBridge_0.2.4_aarch64.app.tar.gz',
          browser_download_url:
            'https://github.com/vergil-lai/print-bridge/releases/download/printbridge-v0.2.4/PrintBridge_0.2.4_aarch64.app.tar.gz',
        },
      ],
    },
  );

  assert.equal(
    result.platforms['darwin-aarch64'].url,
    'https://github.com/vergil-lai/print-bridge/releases/download/printbridge-v0.2.4/PrintBridge_0.2.4_aarch64.app.tar.gz',
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
