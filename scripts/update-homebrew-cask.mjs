import { createHash } from 'node:crypto';
import { createReadStream, readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const VERSION_PATTERN = /^(\s*)version\s+"[^"]+"/gm;
const SHA256_PATTERN = /^(\s*)sha256\s+arm:\s*"[^"]+",\r?\n\s*intel:\s*"[^"]+"/gm;

export function rewriteHomebrewCask(source, { version, armSha256, intelSha256 }) {
  assertSingleMatch(source, VERSION_PATTERN, 'version stanza');
  assertSingleMatch(source, SHA256_PATTERN, 'dual-architecture sha256 stanza');

  const withVersion = source.replace(VERSION_PATTERN, `$1version "${version}"`);

  return withVersion.replace(
    SHA256_PATTERN,
    `$1sha256 arm:   "${armSha256}",\n$1       intel: "${intelSha256}"`,
  );
}

export function findReleaseDmgs(directory) {
  const names = readdirSync(directory).filter((name) => name.toLowerCase().endsWith('.dmg'));
  const armNames = names.filter((name) => /(?:aarch64|arm64).*\.dmg$/i.test(name));
  const intelNames = names.filter((name) => /(?:x86_64|x64|amd64).*\.dmg$/i.test(name));

  assertSingleAsset(armNames, 'Apple Silicon');
  assertSingleAsset(intelNames, 'Intel');

  return {
    armPath: join(directory, armNames[0]),
    intelPath: join(directory, intelNames[0]),
  };
}

export function sha256File(filePath) {
  return new Promise((resolveHash, reject) => {
    const hash = createHash('sha256');
    const stream = createReadStream(filePath);

    stream.on('error', reject);
    stream.on('data', (chunk) => hash.update(chunk));
    stream.on('end', () => resolveHash(hash.digest('hex')));
  });
}

async function main() {
  const options = parseArgs(process.argv.slice(2));
  const caskPath = resolve(options.cask);
  const assetsDirectory = resolve(options.assetsDirectory);
  const { armPath, intelPath } = findReleaseDmgs(assetsDirectory);
  const [armSha256, intelSha256] = await Promise.all([sha256File(armPath), sha256File(intelPath)]);
  const source = readFileSync(caskPath, 'utf8');
  const updated = rewriteHomebrewCask(source, {
    version: options.version,
    armSha256,
    intelSha256,
  });

  writeFileSync(caskPath, updated);
  console.log(`Updated ${caskPath} to PrintBridge ${options.version}.`);
  console.log(`Apple Silicon: ${armSha256}`);
  console.log(`Intel: ${intelSha256}`);
}

function parseArgs(args) {
  const options = {};

  for (let index = 0; index < args.length; index += 2) {
    const flag = args[index];
    const value = args[index + 1];

    if (!value) {
      throw new Error(`Missing value for ${flag ?? 'argument'}.`);
    }

    switch (flag) {
      case '--version':
        options.version = value;
        break;
      case '--assets-dir':
        options.assetsDirectory = value;
        break;
      case '--cask':
        options.cask = value;
        break;
      default:
        throw new Error(`Unknown argument: ${flag}`);
    }
  }

  for (const key of ['version', 'assetsDirectory', 'cask']) {
    if (!options[key]) {
      throw new Error(
        'Usage: update-homebrew-cask.mjs --version VERSION --assets-dir DIR --cask FILE',
      );
    }
  }

  return options;
}

function assertSingleMatch(source, pattern, description) {
  const matches = [...source.matchAll(pattern)];
  if (matches.length !== 1) {
    throw new Error(`Expected exactly one ${description}, found ${matches.length}.`);
  }
}

function assertSingleAsset(names, architecture) {
  if (names.length !== 1) {
    throw new Error(
      `Expected exactly one ${architecture} DMG, found ${names.length}: ${names.join(', ') || 'none'}.`,
    );
  }
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  await main();
}
