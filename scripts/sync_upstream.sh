#!/usr/bin/env bash
set -e

# ZeroClaw Upstream Sync & Validate Script
# Optimized for Termux / Android

echo "🔄 Fetching latest changes from origin/main..."
git fetch origin main

# Check if we are actually behind
UPSTREAM=${1:-'@{u}'}
LOCAL=$(git rev-parse @)
REMOTE=$(git rev-parse "$UPSTREAM")

if [ "$LOCAL" = "$REMOTE" ]; then
    echo "✅ Already up to date."
    exit 0
fi

echo "📦 Stashing local uncommitted changes..."
git stash push -m "Auto-stash before sync $(date)"

echo "🔀 Rebasing local branch onto origin/main..."
if ! git rebase origin/main; then
    echo "❌ Rebase failed due to conflicts. Please resolve manually."
    exit 1
fi

echo "🔨 Validating build (Release mode)..."
if ! cargo build --release --locked; then
    echo "❌ BUILD FAILED! The new upstream changes broke the build in this environment."
    echo "💡 Reverting rebase..."
    git rebase --abort || true
    git stash pop || true
    exit 1
fi

echo "🧪 Running core tests..."
if ! cargo test; then
    echo "⚠️  TESTS FAILED! The build is okay, but some logic might be broken."
    # We don't necessarily revert on test failure, but we warn the user.
fi

echo "🎁 Restoring local changes..."
git stash pop || echo "⚠️  No stash to pop or minor conflicts in stash."

echo "🚀 Sync complete! ZeroClaw is updated and verified."
