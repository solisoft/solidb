#!/usr/bin/env bash
# Transfers between accounts, in transactions, every commit fsynced:
# PostgreSQL against SoliDB, with the machine, limits and images of run.sh.
#
#   ./bank.sh all                     the throughput runs, then the crash runs
#   ./bank.sh postgres solidb-2.9.1   only those
#   WARM=1 SECS=3 RUNS=1 CYCLES=2 ./bank.sh all    smoke pass
#
# Throughput and correctness (solidb-bank, see src/bank/main.rs). Series:
# 10,000 accounts at 1 and 32 clients, 10 accounts at 32 clients.
#   postgres        synchronous_commit=on; modes update (READ COMMITTED,
#                   conditional UPDATEs), serializable (read then write)
#   postgres-naive  the control: read then write at READ COMMITTED, which
#                   loses updates; 10 accounts, 32 clients, one run
#   solidb-2.9.0    the published 2.9.0 image, solistore, SOLIDB_DURABILITY=full;
#                   modes query (SDBQL UPDATE in a transaction), doc
#   solidb-2.9.1    the same on 2.9.1
# Crash: the container is killed with SIGKILL while 32 clients transfer, then
# started again on the same data, CYCLES times; checked after every restart.
#   crash-postgres          synchronous_commit=on
#   crash-postgres-async    synchronous_commit=off (no fsync before the ack)
#   crash-solidb            2.9.1, SOLIDB_DURABILITY=full
#   crash-solidb-eventual   2.9.1, SOLIDB_DURABILITY=eventual (fsync every 100 ms)
set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
DOCKER=${DOCKER:-docker}
SUDO=${SUDO:-sudo}
BIN=${BIN:-$HERE/target/release/solidb-bank}
ROOT=${BENCH_ROOT:-$HOME/bench}
OUT=${OUT:-$ROOT/results/$(date +%F)/bank}
DB_CPUS=${DB_CPUS:-0-3,16-19}
CLIENT_CPUS=${CLIENT_CPUS:-8-15,24-31}
DB_MEM=${DB_MEM:-16g}
PASSWORD=bench
COMMON=(--warm "${WARM:-5}" --secs "${SECS:-20}" --runs "${RUNS:-3}" --seed "${SEED:-1}")
CYCLES=${CYCLES:-20}

POSTGRES_IMAGE=docker.io/library/postgres@sha256:e3d24f6906d0760bf4da0dae872415391ededefad23fd471c47dc53b0ddf3c42 # 18.6
SOLIDB_290=${SOLIDB_290:-ghcr.io/solisoft/solidb@sha256:05a5da7d3961d2358db55453723c18ead98ab809fec4bd4df9d109e07c4f69e5} # v2.9.0
SOLIDB_291=${SOLIDB_291:-ghcr.io/solisoft/solidb@sha256:29f31b15a448fd7c680fb52c4a9e29dd16ea3c3de2eaf9181a17f1df63d94214} # v2.9.1

ALL=(postgres postgres-naive solidb-2.9.0 solidb-2.9.1
  crash-postgres crash-postgres-async crash-solidb crash-solidb-eventual)

mkdir -p "$OUT"
NAME=bench-db

wait_port() {
  local port=$1 i
  for i in $(seq 1 240); do
    if (exec 3<>"/dev/tcp/127.0.0.1/$port") 2>/dev/null; then
      sleep 5
      return 0
    fi
    sleep 0.5
  done
  echo "port $port never opened" >&2
  return 1
}

cgroup_of() {
  local id path
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

# Sets ENGINE, PORT, IMAGE and RUN (the container's command line) for a
# configuration, and MODES / SERIES for its throughput runs.
configure() {
  local cfg=$1 data=$2
  # shellcheck disable=SC2206
  local base=($DOCKER run -d --name "$NAME" --network host --cpuset-cpus "$DB_CPUS"
    --memory "$DB_MEM" --memory-swap "$DB_MEM")
  SERIES=10000:1,10000:32,10:32
  case $cfg in
    postgres | postgres-naive | crash-postgres | crash-postgres-async)
      ENGINE=postgres PORT=5432 IMAGE=$POSTGRES_IMAGE
      RUN=("${base[@]}" --shm-size 1g -e POSTGRES_PASSWORD=$PASSWORD -v "$data:/var/lib/postgresql:Z"
        "$IMAGE" -c shared_buffers=4GB -c effective_cache_size=12GB -c max_connections=200)
      [[ $cfg == crash-postgres-async ]] && RUN+=(-c synchronous_commit=off)
      MODES=(update serializable)
      [[ $cfg == postgres-naive ]] && MODES=(naive) && SERIES=10:32
      [[ $cfg == crash-* ]] && MODES=(update)
      ;;
    solidb-2.9.0 | solidb-2.9.1 | crash-solidb | crash-solidb-eventual)
      ENGINE=solidb PORT=6800 IMAGE=$SOLIDB_291
      [[ $cfg == solidb-2.9.0 ]] && IMAGE=$SOLIDB_290
      RUN=("${base[@]}" -e SOLIDB_ADMIN_PASSWORD=$PASSWORD -e SOLIDB_PORT=$PORT -v "$data:/data:Z,U"
        -e SOLIDB_ENGINE=solistore)
      if [[ $cfg == crash-solidb-eventual ]]; then
        RUN+=(-e SOLIDB_DURABILITY=eventual)
      else
        RUN+=(-e SOLIDB_DURABILITY=full)
      fi
      RUN+=("$IMAGE")
      MODES=(query doc)
      [[ $cfg == crash-* ]] && MODES=(query)
      ;;
    *)
      echo "unknown configuration $cfg" >&2
      return 1
      ;;
  esac
}

run_one() {
  local cfg=$1
  local data=$ROOT/data/$cfg status=0
  $SUDO rm -rf "$data"
  mkdir -p "$data"
  configure "$cfg" "$data"
  $DOCKER rm -f "$NAME" >/dev/null 2>&1 || true
  echo "== $cfg: ${RUN[*]}"
  "${RUN[@]}" >/dev/null
  if ! wait_port "$PORT"; then
    $DOCKER logs "$NAME" >"$OUT/$cfg.server.log" 2>&1 || true
    $DOCKER rm -f "$NAME" >/dev/null
    return 1
  fi
  local digest version cg
  digest=$($DOCKER image inspect --format '{{.Digest}}' "$IMAGE" 2>/dev/null || echo "$IMAGE")
  version=""
  [[ $ENGINE == solidb ]] && version=$($DOCKER exec "$NAME" solidb --version 2>/dev/null || true)
  [[ $ENGINE == postgres ]] && version=$($DOCKER exec "$NAME" postgres --version 2>/dev/null || true)
  local meta=(--meta "container=${RUN[*]}" --meta "image=$IMAGE" --meta "image_digest=$digest"
    --meta "server_version=$version" --meta "db_cpus=$DB_CPUS" --meta "client_cpus=$CLIENT_CPUS"
    --meta "db_memory=$DB_MEM" --meta "host_cpu=$(lscpu | sed -n 's/^Model name: *//p')"
    --meta "host_kernel=$(uname -r)")
  for mode in "${MODES[@]}"; do
    local label=$cfg-$mode
    [[ $cfg == crash-* || $cfg == *-$mode ]] && label=$cfg
    cg=$(cgroup_of)
    set +e
    if [[ $cfg == crash-* ]]; then
      taskset -c "$CLIENT_CPUS" "$BIN" --engine "$ENGINE" --mode "$mode" --label "$label" \
        --addr "127.0.0.1:$PORT" --password $PASSWORD --seed "${SEED:-1}" \
        --crash-cycles "$CYCLES" --accounts 10000 --concurrency 32 \
        --kill "$DOCKER kill -s KILL $NAME >/dev/null" --start "$DOCKER start $NAME >/dev/null" \
        "${meta[@]}" --out "$OUT/$label.json" 2>&1 | tee "$OUT/$label.log"
    else
      taskset -c "$CLIENT_CPUS" "$BIN" --engine "$ENGINE" --mode "$mode" --label "$label" \
        --addr "127.0.0.1:$PORT" --password $PASSWORD --series "$SERIES" "${COMMON[@]}" \
        --db-cgroup "$cg" "${meta[@]}" --out "$OUT/$label.json" 2>&1 | tee "$OUT/$label.log"
    fi
    local s=${PIPESTATUS[0]}
    set -e
    # 1 = the checks found violations (recorded in the JSON); anything
    # else means the run itself failed.
    [[ $s != 0 ]] && status=$s
  done
  $DOCKER logs "$NAME" >"$OUT/$cfg.server.log" 2>&1 || true
  $DOCKER rm -f "$NAME" >/dev/null
  $SUDO rm -rf "$data"
  return "$status"
}

configs=("$@")
[[ ${configs[0]:-all} == all ]] && configs=("${ALL[@]}")
failed=()
for cfg in "${configs[@]}"; do
  run_one "$cfg" || failed+=("$cfg")
done
echo "results in $OUT"
if ((${#failed[@]})); then
  echo "violations or failures: ${failed[*]}" >&2
  exit 1
fi
