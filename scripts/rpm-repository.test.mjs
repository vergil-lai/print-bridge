import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

test('RPM workflow supports initial publication and later stable releases', () => {
  const workflow = readFileSync('.github/workflows/publish-rpm.yml', 'utf8');

  assert.match(workflow, /workflow_dispatch:/);
  assert.match(workflow, /release:\n\s+types:\n\s+- published/);
  assert.match(workflow, /github\.event\.release\.prerelease/);
  assert.match(workflow, /secrets\.CLOUDFLARE_R2_API_TOKEN/);
  assert.match(workflow, /secrets\.CLOUDFLARE_ACCOUNT_ID/);
  assert.match(workflow, /secrets\.APT_GPG_PRIVATE_KEY/);
  assert.match(workflow, /secrets\.APT_GPG_PASSPHRASE/);
  assert.match(workflow, /secrets\.KEY_FINGERPRINT/);
});

test('RPM repository build validates products and architectures before signing metadata', () => {
  const script = readFileSync('scripts/build-rpm-repository.sh', 'utf8');

  for (const packageName of ['print-bridge', 'print-bridge-server']) {
    for (const architecture of ['x86_64', 'aarch64']) {
      assert.ok(script.includes(`require_package "${packageName}" "${architecture}"`));
    }
  }

  assert.match(script, /createrepo_c/);
  assert.match(script, /repodata\/repomd\.xml/);
  assert.match(script, /--detach-sign/);
  assert.match(script, /RPM-GPG-KEY-printbridge/);
  assert.match(script, /printbridge\.repo/);
  assert.match(script, /repo_gpgcheck=1/);
  assert.match(script, /gpgcheck=0/);
});

test('R2 upload uses the rpm prefix and switches repomd.xml last', () => {
  const script = readFileSync('scripts/upload-rpm-repository.sh', 'utf8');
  const packagesIndex = script.indexOf('upload_tree "$ROOT/packages"');
  const signatureIndex = script.indexOf('upload_file "$ROOT/repodata/repomd.xml.asc"');
  const metadataIndex = script.indexOf('upload_file "$ROOT/repodata/repomd.xml"');

  assert.match(script, /R2_PREFIX/);
  assert.match(script, /upload_file "\$ROOT\/printbridge\.repo"/);
  assert.ok(packagesIndex >= 0);
  assert.ok(signatureIndex > packagesIndex);
  assert.ok(metadataIndex > signatureIndex);
  assert.match(script, /wrangler@4 r2 object put .* --remote/);
});
