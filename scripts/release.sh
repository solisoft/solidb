#!/bin/bash
# SolidB Release Helper Script
# Usage: ./scripts/release.sh [patch|minor|major] [--dry-run]
#
# Examples:
#   ./scripts/release.sh patch    # Bump patch version (0.5.0 -> 0.5.1)
#   ./scripts/release.sh minor    # Bump minor version (0.5.0 -> 0.6.0)
#   ./scripts/release.sh major    # Bump major version (0.5.0 -> 1.0.0)
#   ./scripts/release.sh patch --dry-run  # Preview changes without executing

set -e

VERSION_BUMP=${1:-patch}
DRY_RUN=""
EXEC_FLAG="--execute"

# Parse arguments
for arg in "$@"; do
    case $arg in
        --dry-run)
            DRY_RUN="--dry-run"
            EXEC_FLAG="--dry-run"
            shift
            ;;
        patch|minor|major)
            VERSION_BUMP=$arg
            shift
            ;;
    esac
done

echo "=========================================="
echo "SolidB Release - $VERSION_BUMP version bump"
echo "=========================================="

# Check if cargo-release is installed
if ! command -v cargo-release &> /dev/null; then
    echo "Installing cargo-release..."
    cargo install cargo-release
fi

# The docs site is not touched by `cargo release`, so it drifts unless someone
# remembers. Check it up front: failing here costs nothing, whereas noticing
# after the tag and crates.io publish means a follow-up release.
#
# This checks the *current* version. Bump the version pill and add the
# changelog section for the version you are about to release, then re-run.
echo ""
echo "Checking the docs site is in sync..."
if ! ./scripts/check_docs_sync.sh; then
    echo ""
    echo "Refusing to release with a stale docs site." >&2
    echo "Update doc/ for the version being released, then re-run." >&2
    exit 1
fi

# Show current versions
echo ""
echo "Current versions:"
echo "  solidb:        $(grep -m1 '^version' Cargo.toml | cut -d'=' -f2 | tr -d ' ')"
echo "  solidb-client: $(grep -m1 '^version' clients/rust-client/Cargo.toml | cut -d'=' -f2 | tr -d ' ')"

# Run cargo release
echo ""
echo "Running cargo release --workspace $VERSION_BUMP $DRY_RUN..."

if [ -n "$DRY_RUN" ]; then
    cargo release --workspace "$VERSION_BUMP"
else
    echo ""
    echo "This will:"
    echo "  1. Update versions in Cargo.toml files"
    echo "  2. Update Cargo.lock"
    echo "  3. Create git commits for changes"
    echo "  4. Create git tags"
    echo "  5. Publish to crates.io"
    echo ""
    read -p "Proceed? (y/N) " -n 1 -r
    echo
    if [[ $REPLY =~ ^[Yy]$ ]]; then
        cargo release --workspace "$VERSION_BUMP" --no-confirm --execute
    else
        echo "Aborted."
        exit 1
    fi
fi

echo ""
echo "Release complete!"
echo ""
echo "Next steps:"
echo "  1. Push the git tags: git push --tags"
echo "  2. GitHub Actions will create the GitHub release"
echo ""
echo "Or use the --dry-run flag to preview changes."
