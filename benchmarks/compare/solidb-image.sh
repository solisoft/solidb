#!/usr/bin/env bash
# Build the SoliDB image the 2.7.2 figures were taken with: the official
# v2.7.2 release binary, in the repository's own Dockerfile, on a base whose
# glibc is new enough for it. From 2.7.3 the published image works and
# run.sh uses it; this is only for reproducing the 2.7.2 run
# (SOLIDB_IMAGE=localhost/solidb-bench:2.7.2 ./run.sh solidb …).
#
# Why not the published image: ghcr.io/solisoft/solidb:v2.5.0 through v2.7.2
# do not start. The release binary is built on ubuntu-latest (glibc 2.39) and
# the Dockerfile's base, debian:bookworm-slim, ships glibc 2.36, so the binary
# exits with "GLIBC_2.38 not found". The only change here is that base.
#
#   DOCKER="sudo podman" ./solidb-image.sh            # downloads the release asset
#   TARBALL=./solidb-linux-amd64.tar.gz ./solidb-image.sh
set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
DOCKER=${DOCKER:-docker}
VERSION=v2.7.2
# The digest GitHub records for the release asset solidb-linux-amd64.tar.gz.
SHA256=4b81a35a9ffecca17d1970bbd48f7f3b13814adb80fd89699a196dff524400b3
TAG=localhost/solidb-bench:${VERSION#v}

ctx=$(mktemp -d)
trap 'rm -rf "$ctx"' EXIT
if [ -n "${TARBALL:-}" ]; then
  cp "$TARBALL" "$ctx/rel.tar.gz"
else
  curl -sSfL --retry 5 -o "$ctx/rel.tar.gz" \
    "https://github.com/solisoft/solidb/releases/download/$VERSION/solidb-linux-amd64.tar.gz"
fi
echo "$SHA256  $ctx/rel.tar.gz" | sha256sum -c -
mkdir "$ctx/binaries"
tar -xzf "$ctx/rel.tar.gz" -C "$ctx/binaries"
sed 's/^FROM debian:bookworm-slim/FROM debian:trixie-slim/' "$HERE/../../Dockerfile" >"$ctx/Dockerfile"
grep -q '^FROM debian:trixie-slim' "$ctx/Dockerfile"
# shellcheck disable=SC2086
$DOCKER build -q -t "$TAG" "$ctx"
# shellcheck disable=SC2086
$DOCKER run --rm "$TAG" solidb --version
echo "$TAG"
