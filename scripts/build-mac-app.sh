#!/usr/bin/env bash
# Build Mac Storage Advisor.app from the same binary as `mac-storage window`.
# Run this on macOS. Opening the app starts that window. The command line is unchanged.

set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "Build the Mac app on macOS. This machine is $(uname -s)." >&2
  exit 1
fi

if ! cargo tauri --version >/dev/null 2>&1; then
  cargo install tauri-cli --version 2.12.0 --locked
fi

config_args=()
if [[ -n "${APPLE_SIGNING_IDENTITY:-}" ]]; then
  escaped="${APPLE_SIGNING_IDENTITY//\\/\\\\}"
  escaped="${escaped//\"/\\\"}"
  config_args=(--config "{\"bundle\":{\"macOS\":{\"signingIdentity\":\"${escaped}\"}}}")
fi

(
  cd "$root/apps/cli"
  cargo tauri build --bundles app,dmg "${config_args[@]}"
)

bundle_dir="$root/target/release/bundle/macos"
app="$bundle_dir/Mac Storage Advisor.app"
binary="$app/Contents/MacOS/mac-storage"

if [[ ! -x "$binary" ]]; then
  echo "The app bundle is missing its mac-storage binary: $binary" >&2
  exit 1
fi

echo "App: $app"
echo "Open it with: open \"$app\""
if compgen -G "$bundle_dir/*.dmg" >/dev/null; then
  echo "Disk image:"
  ls -1 "$bundle_dir"/*.dmg
fi
echo "The command line inside the app still works:"
echo "  \"$binary\" scan ~/Downloads"
