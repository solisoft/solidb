#!/usr/bin/env bash
# The short pass of the fault harness: a few kill -9 cycles on each engine
# and durability mode, then a few seconds of the isolation workload on each
# path. What the `faults` CI job runs (card #864); the long runs of the
# README are still made by hand.
#
#   faults/ci.sh <solidb binary> [<solidb-faults binary>] [<output dir>]
#
# FAULTS_CYCLES (4), FAULTS_SECS (5) and FAULTS_SEED (random, printed) set
# its length and make a failure repeatable. Exit status 1 when a run found a
# violation or an anomaly, or did not complete; the output directory then
# holds each failed run's report, history and server log.
set -uo pipefail

BIN=${1:?usage: faults/ci.sh <solidb binary> [<solidb-faults binary>] [<output dir>]}
HARNESS=${2:-$(dirname "$0")/target/release/solidb-faults}
OUT=${3:-$(mktemp -d)}
CYCLES=${FAULTS_CYCLES:-4}
SECS=${FAULTS_SECS:-5}
SEED=${FAULTS_SEED:-$((RANDOM + 1))}
PORT=${FAULTS_PORT:-6900}

BIN=$(realpath "$BIN")
HARNESS=$(realpath "$HARNESS")
mkdir -p "$OUT"
OUT=$(realpath "$OUT")
echo "[faults] seed $SEED, $CYCLES cycles, $SECS s of isolation; output in $OUT"

failed=0
run() { # name, then the harness's arguments
  local name=$1
  shift
  if "$HARNESS" "$@" --bin "$BIN" --port "$PORT" --seed "$SEED" \
    --data "$OUT/$name" >"$OUT/$name.report" 2>"$OUT/$name.log"; then
    echo "[faults] ok      $name"
    # Kept only for a run that failed: a history is tens of megabytes.
    rm -rf "$OUT/$name" "$OUT/$name.history.json"
  else
    echo "[faults] FAILED  $name (exit $?)"
    tail -n 20 "$OUT/$name.log"
    failed=1
  fi
}

for config in "rocksdb eventual" "solistore eventual" "solistore full"; do
  read -r engine durability <<<"$config"
  run "crash-$engine-$durability" crash --engine "$engine" --durability "$durability" \
    --cycles "$CYCLES" --min-ms 500 --max-ms 1500 --out "$OUT/crash-$engine-$durability.jsonl"
done

for config in "solistore cursor read_committed" "solistore sdbql read_committed" \
  "solistore sdbql serializable" "solistore doc serializable" "rocksdb sdbql read_committed"; do
  read -r engine mode level <<<"$config"
  name="isolation-$engine-$mode-$level"
  run "$name" isolation --engine "$engine" --mode "$mode" --isolation "$level" \
    --secs "$SECS" --history "$OUT/$name.history.json"
done

if [ "$failed" -ne 0 ]; then
  echo "[faults] at least one run failed; repeat with FAULTS_SEED=$SEED"
  exit 1
fi
echo "[faults] every run passed"
