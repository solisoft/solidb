#!/usr/bin/env bash
# The columnar benchmark: one wide table of events and four analytical
# questions against SoliDB and ClickHouse, one database at a time, each in a
# container with the same cores and memory as run.sh gives.
#
#   ./col.sh all                   every configuration, one after another
#   ROWS=1000000 WARM=1 SECS=5 RUNS=1 ./col.sh all    a smaller pass
#
# Configurations:
#   solidb      SoliDB (solistore engine), a columnar collection, SDBQL
#   clickhouse  ClickHouse, a MergeTree table ordered by (country, ts)
#
# Every answer is checked against the generator's own (src/col/main.rs).
set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
DOCKER=${DOCKER:-docker}
SUDO=${SUDO:-sudo}
BIN=${BIN:-$HERE/target/release/solidb-col}
ROOT=${BENCH_ROOT:-$HOME/bench}
OUT=${OUT:-$ROOT/results/col-$(date +%F)}
DB_CPUS=${DB_CPUS:-0-3,16-19}
CLIENT_CPUS=${CLIENT_CPUS:-8-15,24-31}
DB_MEM=${DB_MEM:-16g}
PASSWORD=bench
ARGS=(--rows "${ROWS:-10000000}" --concurrency "${CONCURRENCY:-1,4}"
  --warm "${WARM:-5}" --secs "${SECS:-20}" --runs "${RUNS:-3}")
[[ -n ${ONLY:-} ]] && ARGS+=(--only "$ONLY")

SOLIDB_IMAGE=${SOLIDB_IMAGE:-ghcr.io/solisoft/solidb:v2.10.4}
# Pinned by digest; the tag is for the reader.
CLICKHOUSE_IMAGE=${CLICKHOUSE_IMAGE:-docker.io/clickhouse/clickhouse-server@sha256:4f491abc44d9f4a1747c033c35ddd5bc385521305d85ca8251ba0824ff597035} # latest, 2026-11-15: 26.10.4

ALL=(solidb clickhouse)
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
  local data=$ROOT/data/col-$cfg
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
    clickhouse)
      engine=clickhouse port=8123 image=$CLICKHOUSE_IMAGE settle=10
      run=("${base[@]}" --ulimit nofile=262144:262144 -e CLICKHOUSE_SKIP_USER_SETUP=1
        -v "$data:/var/lib/clickhouse:Z" "$image")
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

[ $# -gt 0 ] || { sed -n '2,13p' "$0"; exit 1; }
configs=("$@")
[ "$1" = all ] && configs=("${ALL[@]}")
failed=()
for cfg in "${configs[@]}"; do
  run_one "$cfg" || failed+=("$cfg")
done
echo "results in $OUT"
[ ${#failed[@]} -eq 0 ] || { echo "failed: ${failed[*]}" >&2; exit 1; }
