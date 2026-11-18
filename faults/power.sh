#!/usr/bin/env bash
# The crash test under a power loss instead of a process kill (card #863):
# the server's data directory is on LazyFS, a FUSE filesystem that keeps
# what is written in a cache of its own until it is fsynced, and after each
# SIGKILL the harness has it drop that cache. What was not fsynced is gone,
# as after a machine that lost power.
#
#   faults/power.sh <solidb binary> <lazyfs directory> [<output dir>]
#
# <lazyfs directory> holds the `lazyfs` executable and the shared libraries
# it needs (libpcache, and libfuse3 when the system's is another version).
# LazyFS is external code (github.com/dsrhaslab/lazyfs): it is not in this
# repository and nothing here downloads it.
#
# FAULTS_CYCLES (6) and FAULTS_SEED (random, printed) as in ci.sh. Exit
# status 1 when a run found a violation or did not complete.
set -uo pipefail

BIN=$(realpath "${1:?usage: faults/power.sh <solidb binary> <lazyfs directory> [<output dir>]}")
LAZY=$(realpath "${2:?usage: faults/power.sh <solidb binary> <lazyfs directory> [<output dir>]}")
OUT=$(realpath -m "${3:-$(mktemp -d)}")
HARNESS=$(realpath "$(dirname "$0")/target/release/solidb-faults")
CYCLES=${FAULTS_CYCLES:-6}
SEED=${FAULTS_SEED:-$((RANDOM + 1))}
PORT=${FAULTS_PORT:-6900}
mkdir -p "$OUT"
echo "[power] seed $SEED, $CYCLES cycles; output in $OUT"

failed=0
# name, engine, durability, transaction isolation, singles may be lost, transactions may be lost
run() {
  local name=$1 engine=$2 durability=$3 iso=$4 singles=$5 tx=$6
  local dir=$OUT/$name
  mkdir -p "$dir/mnt" "$dir/root"
  cat > "$dir/lazyfs.toml" <<TOML
[faults]
fifo_path="$dir/faults.fifo"
fifo_path_completed="$dir/done.fifo"
[cache]
apply_eviction=false
[cache.simple]
custom_size="2gb"
blocks_per_page=1
[filesystem]
log_all_operations=false
logfile=""
TOML
  LD_LIBRARY_PATH=$LAZY "$LAZY/lazyfs" "$dir/mnt" --config-path "$dir/lazyfs.toml" \
    -o modules=subdir -o subdir="$dir/root" -f > "$dir/lazyfs.log" 2>&1 &
  local lazy=$!
  for _ in $(seq 50); do mountpoint -q "$dir/mnt" && [ -p "$dir/faults.fifo" ] && break; sleep 0.1; done
  if ! mountpoint -q "$dir/mnt"; then
    echo "[power] FAILED  $name: LazyFS did not mount (see $dir/lazyfs.log)"; failed=1; return
  fi
  "$HARNESS" crash --bin "$BIN" --engine "$engine" --durability "$durability" --tx-isolation "$iso" \
    --cycles "$CYCLES" --seed "$SEED" --port "$PORT" --data "$dir/mnt/data" \
    --lazyfs-fifo "$dir/faults.fifo" --lazyfs-done "$dir/done.fifo" \
    --singles-may-be-lost "$singles" --tx-may-be-lost "$tx" \
    --out "$dir/crash.jsonl" > "$dir/harness.log" 2>&1
  local status=$?
  cp "$dir/mnt/data.server.log" "$dir/server.log" 2>/dev/null
  fusermount3 -u "$dir/mnt" 2>/dev/null || fusermount3 -uz "$dir/mnt"
  wait "$lazy" 2>/dev/null
  if [ $status -eq 0 ]; then echo "[power] ok      $name"; else echo "[power] FAILED  $name (see $dir)"; failed=1; fi
}

#   every commit and every insert is fsynced before its answer: nothing may be lost
run solistore-full                solistore full     read_committed   0 0
#   single inserts are fsynced in the background, a transaction's commit before its answer
run solistore-eventual            solistore eventual read_committed   1 0
#   neither is fsynced before its answer
run solistore-eventual-unsynced   solistore eventual read_uncommitted 1 1
run rocksdb-eventual              rocksdb   eventual read_committed   1 0

[ $failed -eq 0 ] && echo "[power] every run passed"
exit $failed
