#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
DEST=${1:-"$HOME/.local/bin/herdr"}

cd "$ROOT_DIR"
just build

for name in $(herdr session list --json | jq -r '.sessions[] | select(.running) | .name'); do
  herdr session stop "$name"
done

mkdir -p "$(dirname -- "$DEST")"
cp target/release/herdr "$DEST"

echo "installed herdr ($(git rev-parse --short HEAD)) to $DEST"
