#!/usr/bin/env bash
# Boot the docs site (doc/) with a given soli binary and require /up -> 200.
#
#   .github/scripts/www-boot-check.sh <path-to-soli> [port]
#
# Boots exactly as the server will: no .env (it is gitignored and lives on the
# server). Booting warms handlers and class methods, so a syntax error, a
# missing view or a broken route registration fails here rather than in
# production. A database is not needed to answer /up.
set -euo pipefail

SOLI=$1
PORT=${2:-19555}
cd "$(dirname "$0")/../../doc"

"$SOLI" serve . --port "$PORT" --workers 1 > /tmp/boot-$PORT.log 2>&1 &
pid=$!
code=000
for _ in $(seq 1 45); do
  code="$(curl -s -o /dev/null -w '%{http_code}' -m 2 "http://127.0.0.1:$PORT/up" || echo 000)"
  [ "$code" = "200" ] && break
  kill -0 $pid 2>/dev/null || { echo "the server exited before answering" >&2; break; }
  sleep 1
done
kill -TERM $pid 2>/dev/null || true
wait $pid 2>/dev/null || true
if [ "$code" != "200" ]; then
  echo "/up answered $code, not 200, under $("$SOLI" --version):" >&2
  tail -40 /tmp/boot-$PORT.log >&2
  exit 1
fi
echo "/up -> 200 under $("$SOLI" --version)"
grep -E 'warmed [0-9]+ handlers' /tmp/boot-$PORT.log || true
