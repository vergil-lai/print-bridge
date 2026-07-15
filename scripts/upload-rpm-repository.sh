#!/bin/sh
set -eu

ROOT=${1:?Usage: upload-rpm-repository.sh REPOSITORY_DIR}
: "${R2_BUCKET:?R2_BUCKET is required}"
: "${R2_PREFIX:?R2_PREFIX is required}"

command -v npx >/dev/null 2>&1 || {
  echo "Required command not found: npx" >&2
  exit 1
}

upload_file() {
  file=$1
  relative_key=${file#"$ROOT"/}
  key="$R2_PREFIX/$relative_key"

  echo "Uploading $key"
  npx --yes wrangler@4 r2 object put "$R2_BUCKET/$key" --file "$file" --remote
}

upload_tree() {
  directory=$1

  find "$directory" -type f | LC_ALL=C sort | while IFS= read -r file; do
    upload_file "$file"
  done
}

upload_repository_data() {
  find "$ROOT/repodata" -type f \
    ! -name 'repomd.xml' \
    ! -name 'repomd.xml.asc' \
    | LC_ALL=C sort \
    | while IFS= read -r file; do
        upload_file "$file"
      done
}

# Upload packages and content-addressed metadata before switching the repository index.
upload_file "$ROOT/printbridge.repo"
upload_file "$ROOT/RPM-GPG-KEY-printbridge"
upload_tree "$ROOT/packages"
upload_repository_data
upload_file "$ROOT/repodata/repomd.xml.asc"
upload_file "$ROOT/repodata/repomd.xml"

echo "Published RPM repository to R2 bucket $R2_BUCKET under $R2_PREFIX/."
