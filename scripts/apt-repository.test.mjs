import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

test('APT workflow supports initial publication and later stable releases', () => {
  const workflow = readFileSync('.github/workflows/publish-apt.yml', 'utf8');

  assert.match(workflow, /workflow_dispatch:/);
  assert.match(workflow, /release:\n\s+types:\n\s+- published/);
  assert.match(workflow, /github\.event\.release\.prerelease/);
  assert.match(workflow, /secrets\.CLOUDFLARE_R2_API_TOKEN/);
  assert.match(workflow, /secrets\.CLOUDFLARE_ACCOUNT_ID/);
  assert.match(workflow, /secrets\.APT_GPG_PRIVATE_KEY/);
  assert.match(workflow, /secrets\.APT_GPG_PASSPHRASE/);
  assert.match(workflow, /secrets\.KEY_FINGERPRINT/);
});

test('APT repository build validates the full product and architecture matrix', () => {
  const script = readFileSync('scripts/build-apt-repository.sh', 'utf8');

  for (const packageName of ['print-bridge', 'print-bridge-server']) {
    for (const architecture of ['amd64', 'arm64']) {
      assert.ok(script.includes(`require_package "${packageName}" "${architecture}"`));
    }
  }

  assert.match(script, /apt-ftparchive --arch/);
  assert.match(script, /Acquire-By-Hash/);
  assert.match(script, /by-hash\/SHA256/);
  assert.match(script, /--clearsign/);
  assert.match(script, /--detach-sign/);
});

test('R2 upload publishes immutable files before signed mutable metadata', () => {
  const script = readFileSync('scripts/upload-apt-repository.sh', 'utf8');
  const poolIndex = script.indexOf('upload_tree "$ROOT/pool"');
  const releaseIndex = script.indexOf('upload_file "$ROOT/dists/stable/Release"');
  const inReleaseIndex = script.indexOf('upload_file "$ROOT/dists/stable/InRelease"');

  assert.ok(poolIndex >= 0);
  assert.ok(releaseIndex > poolIndex);
  assert.ok(inReleaseIndex > releaseIndex);
  assert.match(script, /wrangler@4 r2 object put .* --remote/);
});
