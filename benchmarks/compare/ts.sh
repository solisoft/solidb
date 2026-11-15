#!/usr/bin/env bash
# The time series benchmark: the same points and questions against SoliDB,
# TimescaleDB and QuestDB, one database at a time, each in a container with
# the same cores and memory as run.sh gives.
#
#   ./ts.sh all                    every configuration, one after another
#   ./ts.sh solidb questdb         only those
#   SERIES=100 HOURS=2 WARM=1 SECS=3 RUNS=1 ./ts.sh all    smoke pass
#
# Configurations:
#   solidb      SoliDB (solistore engine), a timeseries collection with
#               column blocks on (ts, series, v), SDBQL. SOLIDB_TS_BLOCKS=0
#               runs it without the blocks.
#   timescale   TimescaleDB, a hypertable, prepared statements
#   questdb     QuestDB over its PostgreSQL wire protocol
#
# Every answer is checked against the generator's own arithmetic
# (src/ts/main.rs); a scenario whose first answers are wrong is skipped and
# reported as such.
set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
DOCKER=${DOCKER:-docker}
SUDO=${SUDO:-sudo}
BIN=${BIN:-$HERE/target/release/solidb-ts}
ROOT=${BENCH_ROOT:-$HOME/bench}
OUT=${OUT:-$ROOT/results/ts-$(date +%F)}
DB_CPUS=${DB_CPUS:-0-3,16-19}
CLIENT_CPUS=${CLIENT_CPUS:-8-15,24-31}
DB_MEM=${DB_MEM:-16g}
PASSWORD=bench
ARGS=(--series "${SERIES:-1000}" --hours "${HOURS:-24}" --concurrency "${CONCURRENCY:-1,8}"
  --warm "${WARM:-5}" --secs "${SECS:-20}" --runs "${RUNS:-3}")
[[ -n ${ONLY:-} ]] && ARGS+=(--only "$ONLY")

SOLIDB_IMAGE=${SOLIDB_IMAGE:-ghcr.io/solisoft/solidb:v2.10.4}
# Pinned by digest; the tag is for the reader.
TIMESCALE_IMAGE=${TIMESCALE_IMAGE:-docker.io/timescale/timescaledb@sha256:6a6b7d7a346d7c6be03a310c240706f3bbc6d7adbdf3fbc07823570b34337f5e} # latest-pg17, 2026-11-15: 2.32.0 on PostgreSQL 17.11
QUESTDB_IMAGE=${QUESTDB_IMAGE:-docker.io/questdb/questdb@sha256:bbf5b3cb0dfb08ad667a008dbcc48794bf942d18d1ad832cb6ecb7ff87314694}             # latest, 2026-11-15: 10.2.0

ALL=(solidb timescale questdb)
mkdir -p "$OUT"
NAME=bench-db

wait_port() {
  local port=$1 i
  for i in $(seq 1 240); do
    if (exec 3<>"/dev/tcp/127.0.0.1/$port") 2>/dev/null; then
      sleep "${2:-5}"
      return 0
    fi
    sleep 0.5
  done
  echo "port $port never opened" >&2
  return 1
}

cgroup_of() {
  local id path p
  id=$($DOCKER inspect --format '{{.Id}}' "$NAME")
  path=$($DOCKER inspect --format '{{.State.CgroupPath}}' "$NAME" 2>/dev/null || true)
  for p in "/sys/fs/cgroup$path" "/sys/fs/cgroup/machine.slice/libpod-$id.scope/container" \
    "/sys/fs/cgroup/machine.slice/libpod-$id.scope" "/sys/fs/cgroup/system.slice/docker-$id.scope"; do
    if [ -f "$p/cpu.stat" ]; then
      echo "$p"
      return
    fi
  done
  echo ""
}

run_one() {
  local cfg=$1
  local data=$ROOT/data/ts-$cfg
  local engine port image settle=5 run=()
  $SUDO rm -rf "$data"
  mkdir -p "$data"
  # shellcheck disable=SC2206
  local base=($DOCKER run -d --name "$NAME" --network host --cpuset-cpus "$DB_CPUS"
    --memory "$DB_MEM" --memory-swap "$DB_MEM")
  case $cfg in
    solidb)
      engine=solidb port=6800 image=$SOLIDB_IMAGE
      run=("${base[@]}" -e SOLIDB_ADMIN_PASSWORD=$PASSWORD -e SOLIDB_PORT=$port -e SOLIDB_ENGINE=solistore
        -e SOLIDB_WRITE_BUFFER_SIZE="${SOLIDB_WRITE_BUFFER_SIZE:-2GB}" -v "$data:/data:Z,U" "$image")
      ;;
    timescale)
      engine=timescale port=5432 image=$TIMESCALE_IMAGE settle=10
      run=("${base[@]}" --shm-size 1g -e POSTGRES_PASSWORD=$PASSWORD
        -v "$data:/var/lib/postgresql/data:Z" "$image" -c shared_buffers=4GB
        -c effective_cache_size=12GB -c max_connections=200 -c synchronous_commit=off)
      ;;
    questdb)
      engine=questdb port=8812 image=$QUESTDB_IMAGE settle=10
      run=("${base[@]}" -v "$data:/var/lib/questdb:Z" "$image")
      ;;
    *)
      echo "unknown configuration $cfg" >&2
      return 1
      ;;
  esac

  $DOCKER rm -f "$NAME" >/dev/null 2>&1 || true
  echo "== $cfg: ${run[*]}"
  "${run[@]}" >/dev/null
  if ! wait_port "$port" "$settle"; then
    $DOCKER logs "$NAME" >"$OUT/$cfg.server.log" 2>&1 || true
    $DOCKER rm -f "$NAME" >/dev/null
    return 1
  fi
  local cgroup status=0
  cgroup=$(cgroup_of)
  $DOCKER inspect "$NAME" >"$OUT/$cfg.container.json"
  taskset -c "$CLIENT_CPUS" "$BIN" --engine "$engine" --addr "127.0.0.1:$port" --password "$PASSWORD" \
    "${ARGS[@]}" ${cgroup:+--cgroup "$cgroup"} --out "$OUT/$cfg.json" \
    2>&1 | tee "$OUT/$cfg.log" || status=$?
  $SUDO du -sb "$data" 2>/dev/null | cut -f1 >"$OUT/$cfg.disk_bytes" || true
  $DOCKER logs "$NAME" >"$OUT/$cfg.server.log" 2>&1 || true
  $DOCKER rm -f "$NAME" >/dev/null
  $SUDO rm -rf "$data"
  return $status
}

[ $# -gt 0 ] || { sed -n '2,19p' "$0"; exit 1; }
configs=("$@")
[ "$1" = all ] && configs=("${ALL[@]}")
failed=()
for cfg in "${configs[@]}"; do
  run_one "$cfg" || failed+=("$cfg")
done
echo "results in $OUT"
[ ${#failed[@]} -eq 0 ] || { echo "failed: ${failed[*]}" >&2; exit 1; }
