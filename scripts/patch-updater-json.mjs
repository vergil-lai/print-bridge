import { spawnSync } from 'node:child_process';
import { mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const DEFAULT_REPOSITORY = 'vergil-lai/print-bridge';
const TAG_PREFIX = 'printbridge-v';
const GITHUB_ASSET_URL_PATTERN =
  /^https:\/\/api\.github\.com\/repos\/[^/]+\/[^/]+\/releases\/assets\/(\d+)$/;

export function rewriteUpdaterAssetUrls(updaterJson, { assets }) {
  const assetUrlsById = new Map(
    assets.map((asset) => [String(asset.id), asset.browser_download_url]),
  );
  const nextJson = structuredClone(updaterJson);

  for (const platform of Object.values(nextJson.platforms ?? {})) {
    if (!platform || typeof platform.url !== 'string') continue;

    const assetId = platform.url.match(GITHUB_ASSET_URL_PATTERN)?.[1];
    if (!assetId) continue;

    const browserDownloadUrl = assetUrlsById.get(assetId);
    if (!browserDownloadUrl) {
      throw new Error(`Could not find release asset ${assetId} for updater URL.`);
    }

    platform.url = browserDownloadUrl;
  }

  return nextJson;
}

export function rewriteUpdaterReleaseNotes(updaterJson, releaseBody) {
  const nextJson = structuredClone(updaterJson);
  nextJson.notes = releaseBody;
  return nextJson;
}

export function findReleaseByTag(releasePages, tagName) {
  const release = releasePages.flat().find((candidate) => candidate.tag_name === tagName);
  if (!release) {
    throw new Error(`Could not find release ${tagName}.`);
  }

  return release;
}

async function main() {
  const repository = process.env.GITHUB_REPOSITORY || DEFAULT_REPOSITORY;
  const tagName = process.argv[2] || `${TAG_PREFIX}${readPackageVersion()}`;
  const tmpDirectory = mkdtempSync(join(tmpdir(), 'printbridge-updater-'));
  const latestJsonPath = join(tmpDirectory, 'latest.json');

  run('gh', [
    'release',
    'download',
    tagName,
    '--repo',
    repository,
    '--pattern',
    'latest.json',
    '--output',
    latestJsonPath,
  ]);

  const releasePages = JSON.parse(
    run('gh', [
      'api',
      '--paginate',
      '--slurp',
      `repos/${repository}/releases?per_page=100`,
    ]).stdout,
  );
  const release = findReleaseByTag(releasePages, tagName);
  const updaterJson = JSON.parse(readFileSync(latestJsonPath, 'utf8'));
  const updaterJsonWithUrls = rewriteUpdaterAssetUrls(updaterJson, {
    assets: release.assets ?? [],
  });
  const patchedJson = rewriteUpdaterReleaseNotes(updaterJsonWithUrls, release.body ?? '');

  writeFileSync(latestJsonPath, `${JSON.stringify(patchedJson, null, 2)}\n`);
  run('gh', ['release', 'upload', tagName, latestJsonPath, '--repo', repository, '--clobber'], {
    stdio: 'inherit',
  });

  console.log(`Patched updater JSON for ${repository}@${tagName}.`);
}

function readPackageVersion() {
  return JSON.parse(readFileSync('apps/desktop/package.json', 'utf8')).version;
}

function run(command, commandArgs, options = {}) {
  const result = spawnSync(command, commandArgs, {
    cwd: process.cwd(),
    env: process.env,
    encoding: 'utf8',
    stdio: options.stdio ?? 'pipe',
  });

  if (result.error) {
    throw new Error(`${command} ${commandArgs.join(' ')} failed: ${result.error.message}`);
  }

  if (result.status !== 0) {
    const stderr = typeof result.stderr === 'string' ? result.stderr.trim() : '';
    throw new Error(`${command} ${commandArgs.join(' ')} failed${stderr ? `:\n${stderr}` : ''}`);
  }

  return result;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  await main();
}
