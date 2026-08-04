#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
WORKTREE_DIR=${HERDR_BUILD_WORKTREE:-"$ROOT_DIR/../herdr-worktrees/build-release"}
DEST=${1:-"$HOME/.local/bin/herdr"}

if [[ -d "$WORKTREE_DIR" ]]; then
  git -C "$WORKTREE_DIR" fetch origin master
  git -C "$WORKTREE_DIR" reset --hard origin/master
else
  git -C "$ROOT_DIR" fetch origin master
  git -C "$ROOT_DIR" worktree add "$WORKTREE_DIR" origin/master
fi

cd "$WORKTREE_DIR"
just build

mkdir -p "$(dirname -- "$DEST")"
cp target/release/herdr "$DEST"

echo "installed herdr ($(git -C "$WORKTREE_DIR" rev-parse --short HEAD)) to $DEST"
