#!/bin/sh
set -eu

ROOT=${1:?Usage: upload-apt-repository.sh REPOSITORY_DIR}
: "${R2_BUCKET:?R2_BUCKET is required}"

command -v npx >/dev/null 2>&1 || {
  echo "Required command not found: npx" >&2
  exit 1
}

upload_file() {
  file=$1
  key=${file#"$ROOT"/}

  echo "Uploading $key"
  npx --yes wrangler@4 r2 object put "$R2_BUCKET/$key" --file "$file" --remote
}

upload_tree() {
  directory=$1

  find "$directory" -type f | LC_ALL=C sort | while IFS= read -r file; do
    upload_file "$file"
  done
}

# Publish immutable objects and index targets before switching the signed release metadata.
upload_file "$ROOT/printbridge-archive-keyring.gpg"
upload_file "$ROOT/printbridge-archive-keyring.asc"
upload_tree "$ROOT/pool"
upload_tree "$ROOT/dists/stable/main"
upload_file "$ROOT/dists/stable/Release"
upload_file "$ROOT/dists/stable/Release.gpg"
upload_file "$ROOT/dists/stable/InRelease"

echo "Published APT repository to R2 bucket $R2_BUCKET."
