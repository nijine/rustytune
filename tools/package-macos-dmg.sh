#!/usr/bin/env bash
# Package the Tauri .app without create-dmg's fragile Finder automation.
set -euo pipefail

repo_dir="$(cd "$(dirname "$0")/.." && pwd)"
app_path="$repo_dir/target/release/bundle/macos/RustyTune.app"

if [[ ! -d "$app_path" ]]; then
  echo "missing app bundle: $app_path" >&2
  echo "run the Tauri app bundle build first" >&2
  exit 1
fi

version="$(awk '
  /^\[workspace.package\]$/ { workspace = 1; next }
  /^\[/ { workspace = 0 }
  workspace && /^version = / { gsub(/[\"[:space:]]/, "", $3); print $3; exit }
' "$repo_dir/Cargo.toml")"

case "$(uname -m)" in
  arm64) bundle_arch="aarch64" ;;
  x86_64) bundle_arch="x64" ;;
  *) bundle_arch="$(uname -m)" ;;
esac

output_dir="$repo_dir/target/release/bundle/dmg"
output_path="$output_dir/RustyTune_${version}_${bundle_arch}.dmg"
staging_dir="$(mktemp -d "${TMPDIR:-/tmp}/rustytune-dmg.XXXXXX")"
hybrid_path="${staging_dir}.hybrid.dmg"

cleanup() {
  rm -rf "$staging_dir"
  rm -f "$hybrid_path"
}
trap cleanup EXIT

mkdir -p "$output_dir"
ditto "$app_path" "$staging_dir/RustyTune.app"
ln -s /Applications "$staging_dir/Applications"

hdiutil makehybrid \
  -hfs \
  -hfs-volume-name RustyTune \
  -o "$hybrid_path" \
  "$staging_dir"

hdiutil convert "$hybrid_path" \
  -format UDZO \
  -ov \
  -o "$output_path"

echo "Bundled $output_path"
