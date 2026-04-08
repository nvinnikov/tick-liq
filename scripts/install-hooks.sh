#!/usr/bin/env bash
# Point git at the repo's versioned hooks directory.
#
# This is per-clone state (stored in .git/config), so every contributor
# needs to run it once after cloning. Running it multiple times is a
# no-op.

set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

git config core.hooksPath .githooks
chmod +x .githooks/*

echo "Installed git hooks: core.hooksPath = .githooks"
echo "Hooks active:"
ls .githooks | sed 's/^/  - /'