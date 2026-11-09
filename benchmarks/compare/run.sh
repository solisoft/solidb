#!/usr/bin/env bash
# Start each database in a container with the same limits, run the harness
# against it, record what it measured, then stop it and wipe its data.
#
#   ./run.sh all                  every configuration below, one after another
#   ./run.sh solidb postgres      only those
#   DOCS=10000 CUSTOMERS=1000 WARM=1 SECS=2 RUNS=1 ./run.sh all    smoke pass
#
# Configurations (all with no fsync on every commit, unless named -durable):
#   solidb                    SoliDB 2.7.3, RocksDB engine (by name only, not in `all`)
#   solidb-solistore          SoliDB 2.7.3, solistore engine
#   postgres                  PostgreSQL 18.6, one jsonb column per document
#   postgres-typed            PostgreSQL 18.6, real columns
#   mongo                     MongoDB 9.0.4
#   arango                    ArangoDB 3.12.4
#   surreal                   SurrealDB 4.0.0
# Durable writes (load, insert and update only; each engine fsyncs a commit
# before it acknowledges it):
#   solidb-solistore-durable  SOLIDB_DURABILITY=full
#   postgres-durable          synchronous_commit=on (PostgreSQL's default)
#   mongo-durable             write concern {w: 1, j: true}
#   arango-durable            waitForSync on both collections
#   surreal-durable           sync=every (SurrealDB's default)
#
# Written for Linux with Docker (DOCKER=docker, the default) or rootful
# podman (DOCKER="sudo podman"). The data directories belong to the
# containers' users, so du and rm go through $SUDO.
set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
DOCKER=${DOCKER:-docker}
SUDO=${SUDO:-sudo}
BIN=${BIN:-$HERE/target/release/solidb-compare}
ROOT=${BENCH_ROOT:-$HOME/bench}
OUT=${OUT:-$ROOT/results/$(date +%F)}
# Four physical cores (and their SMT siblings) for the database, eight others
# for the harness. Adjust to the machine: `lscpu -e=CPU,CORE`.
DB_CPUS=${DB_CPUS:-0-3,16-19}
CLIENT_CPUS=${CLIENT_CPUS:-8-15,24-31}
DB_MEM=${DB_MEM:-16g}
PASSWORD=bench
HARNESS_ARGS=(--docs "${DOCS:-1000000}" --customers "${CUSTOMERS:-100000}" --concurrency "${CONCURRENCY:-1,32}"
  --warm "${WARM:-5}" --secs "${SECS:-20}" --runs "${RUNS:-3}")

# Pinned by digest; the tag is for the reader. The 2.7.2 figures were taken
# with SOLIDB_IMAGE=localhost/solidb-bench:2.7.2, built by ./solidb-image.sh:
# the published v2.5.0-v2.7.2 images do not start (fixed in 2.7.3).
SOLIDB_IMAGE=${SOLIDB_IMAGE:-ghcr.io/solisoft/solidb@sha256:05a5da7d3961d2358db55453723c18ead98ab809fec4bd4df9d109e07c4f69e5} # v2.9.0
POSTGRES_IMAGE=docker.io/library/postgres@sha256:e3d24f6906d0760bf4da0dae872415391ededefad23fd471c47dc53b0ddf3c42  # 18.6
MONGO_IMAGE=docker.io/library/mongo@sha256:452bdaf2127b8ff851e31410a267809b9829eded219979f2210e7c7efb85a058     # 9.0.4
ARANGO_IMAGE=docker.io/arangodb/arangodb@sha256:258323b83ed5cd5bacc6e2d7574777db5a8a61ec75f663e4d690824c3301c7c2 # 3.12.4-3
SURREAL_IMAGE=docker.io/surrealdb/surrealdb@sha256:c35893410644e65fa90aa1bef446ac9cb0a0c2733b9d405ce3eb56754b85c330 # v4.0.0

# SoliDB is compared on its solistore engine only (decided 2026-11-08); the
# RocksDB configuration `solidb` stays available by name.
ALL=(solidb-solistore postgres postgres-typed mongo arango surreal
  solidb-solistore-durable postgres-durable mongo-durable arango-durable surreal-durable)

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

run_one() {
  local cfg=$1
  local data=$ROOT/data/$cfg
  local engine port image phases=() extra=() run=()
  $SUDO rm -rf "$data"
  mkdir -p "$data"
  # $DOCKER unquoted on purpose: "sudo podman" is two words.
  # shellcheck disable=SC2206
  local base=($DOCKER run -d --name "$NAME" --network host --cpuset-cpus "$DB_CPUS"
    --memory "$DB_MEM" --memory-swap "$DB_MEM")
  case $cfg in
    solidb | solidb-solistore | solidb-solistore-durable)
      engine=solidb port=6800 image=$SOLIDB_IMAGE
      run=("${base[@]}" -e SOLIDB_ADMIN_PASSWORD=$PASSWORD -e SOLIDB_PORT=$port -v "$data:/data:Z,U")
      [[ $cfg == solidb-solistore* ]] && run+=(-e SOLIDB_ENGINE=solistore)
      [[ $cfg == *-durable ]] && run+=(-e SOLIDB_DURABILITY=full)
      run+=("$image")
      ;;
    postgres | postgres-typed | postgres-durable)
      engine=postgres port=5432 image=$POSTGRES_IMAGE
      run=("${base[@]}" --shm-size 1g -e POSTGRES_PASSWORD=$PASSWORD -v "$data:/var/lib/postgresql:Z"
        "$image" -c shared_buffers=4GB -c effective_cache_size=12GB -c max_connections=200)
      [[ $cfg != *-durable ]] && run+=(-c synchronous_commit=off)
      [[ $cfg == postgres-typed ]] && extra+=(--typed)
      ;;
    mongo | mongo-durable)
      engine=mongo port=27017 image=$MONGO_IMAGE
      run=("${base[@]}" -v "$data:/data/db:Z" "$image")
      ;;
    arango | arango-durable)
      engine=arango port=8529 image=$ARANGO_IMAGE
      # ArangoDB sizes its caches from the host's memory and cores unless
      # told the container's; these are its documented overrides.
      run=("${base[@]}" -e ARANGO_ROOT_PASSWORD=$PASSWORD -e ARANGODB_OVERRIDE_DETECTED_TOTAL_MEMORY=16G
        -e ARANGODB_OVERRIDE_DETECTED_NUMBER_OF_CORES=8 -v "$data:/var/lib/arangodb3:Z" "$image")
      ;;
    surreal | surreal-durable)
      engine=surreal port=8000 image=$SURREAL_IMAGE
      local path="surrealkv:///data/db?sync=never"
      [[ $cfg == *-durable ]] && path="surrealkv:///data/db"
      run=("${base[@]}" -v "$data:/data:Z,U" "$image" start --user root --pass $PASSWORD
        --bind 0.0.0.0:$port "$path")
      ;;
    *)
      echo "unknown configuration $cfg" >&2
      return 1
      ;;
  esac
  [[ $cfg == *-durable ]] && extra+=(--durable) && phases=(--phases load,insert,update)

  $DOCKER rm -f "$NAME" >/dev/null 2>&1 || true
  echo "== $cfg: ${run[*]}"
  "${run[@]}" >/dev/null
  if ! wait_port $port; then
    $DOCKER logs "$NAME" >"$OUT/$cfg.server.log" 2>&1 || true
    $DOCKER rm -f "$NAME" >/dev/null
    return 1
  fi
  local cg digest version status attempt=0 resume=()
  digest=$($DOCKER image inspect --format '{{.Digest}}' "$image" 2>/dev/null || echo "$image")
  version=""
  [[ $engine == solidb ]] && version=$($DOCKER exec "$NAME" solidb --version 2>/dev/null || true)
  : >"$OUT/$cfg.log"
  : >"$OUT/$cfg.container.json"

  # A series that takes the database down (killed for memory, crashed)
  # makes the harness stop with status 3. The container's state is
  # recorded, the database restarted on the same data, and the harness
  # resumes after that series - at most five times.
  while :; do
    cg=$(cgroup_of)
    set +e
    taskset -c "$CLIENT_CPUS" "$BIN" --engine "$engine" --label "$cfg" --addr "127.0.0.1:$port" \
      --password $PASSWORD "${HARNESS_ARGS[@]}" "${extra[@]}" "${phases[@]}" "${resume[@]}" \
      --db-cgroup "$cg" \
      --after-load "$SUDO du -sb $data | cut -f1" --after-all "$SUDO du -sb $data | cut -f1" \
      --meta "container=${run[*]}" --meta "image=$image" --meta "image_digest=$digest" \
      --meta "server_version=$version" --meta "db_cpus=$DB_CPUS" --meta "client_cpus=$CLIENT_CPUS" \
      --meta "db_memory=$DB_MEM" --meta "host_cpu=$(lscpu | sed -n 's/^Model name: *//p')" \
      --meta "host_kernel=$(uname -r)" --meta "host_memory=$(free -g | awk '/^Mem/{print $2" GB"}')" \
      --out "$OUT/$cfg.json" 2>&1 | tee -a "$OUT/$cfg.log"
    status=${PIPESTATUS[0]}
    set -e
    $DOCKER logs "$NAME" >"$OUT/$cfg.server.$attempt.log" 2>&1 || true
    # One line per attempt: was the database still up, killed (OOM) or crashed?
    $DOCKER inspect "$NAME" --format \
      "{\"attempt\": $attempt, \"harness_status\": $status, \"status\": \"{{.State.Status}}\", \"oom_killed\": {{.State.OOMKilled}}, \"exit_code\": {{.State.ExitCode}}}" \
      >>"$OUT/$cfg.container.json" 2>/dev/null || true
    $DOCKER rm -f "$NAME" >/dev/null
    if [[ $status != 3 || $attempt -ge 5 ]]; then
      break
    fi
    attempt=$((attempt + 1))
    echo "== $cfg: restarting the database on the same data (attempt $attempt)" | tee -a "$OUT/$cfg.log"
    "${run[@]}" >/dev/null
    if ! wait_port $port; then
      $DOCKER logs "$NAME" >"$OUT/$cfg.server.$attempt.log" 2>&1 || true
      $DOCKER rm -f "$NAME" >/dev/null
      break
    fi
    resume=(--resume)
  done
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
  echo "failed: ${failed[*]}" >&2
  exit 1
fi
