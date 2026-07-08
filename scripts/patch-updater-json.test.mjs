import test from 'node:test';
import assert from 'node:assert/strict';

import { rewriteUpdaterAssetUrls } from './patch-updater-json.mjs';

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
