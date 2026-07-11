import { pathToFileURL } from 'node:url';

/** Return whether a SemVer version has a prerelease component. */
export function isPrerelease(version) {
  return version.includes('-');
}

/** Convert a SemVer prerelease separator to Linux package ordering syntax. */
export function toLinuxPackageVersion(version) {
  return version.replace('-', '~');
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const [mode, version] = process.argv.slice(2);

  if (!version || !['linux', 'prerelease'].includes(mode)) {
    console.error('Usage: node scripts/release-version.mjs <linux|prerelease> <version>');
    process.exit(1);
  }

  console.log(mode === 'linux' ? toLinuxPackageVersion(version) : isPrerelease(version));
}
