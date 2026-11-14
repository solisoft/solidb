#!/usr/bin/env bash
# The graph benchmark: the same people, edges and questions against SoliDB,
# Neo4j, Memgraph and ArangoDB, one database at a time, each in
# a container with the same cores and memory as run.sh gives.
#
#   ./graph.sh all                 every configuration, one after another
#   ./graph.sh solidb neo4j        only those
#   PEOPLE=20000 PRODUCTS=500 WARM=1 SECS=3 RUNS=1 ./graph.sh all    smoke pass
#
# Configurations:
#   solidb      SoliDB (solistore engine), SDBQL traversals and SHORTEST_PATH
#   neo4j       Neo4j Community, Cypher over Bolt
#   memgraph    Memgraph Community, Cypher over Bolt (in memory, snapshots off)
#   arango      ArangoDB, AQL traversals
#   postgres    PostgreSQL, joins and a recursive CTE (by name only, not in `all`)
#
# Every answer is checked against the generator's own copy of the graph
# (src/graph/data.rs); a scenario whose first answers are wrong is skipped
# and reported as such.
set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
DOCKER=${DOCKER:-docker}
SUDO=${SUDO:-sudo}
BIN=${BIN:-$HERE/target/release/solidb-graph}
ROOT=${BENCH_ROOT:-$HOME/bench}
OUT=${OUT:-$ROOT/results/graph-$(date +%F)}
DB_CPUS=${DB_CPUS:-0-3,16-19}
CLIENT_CPUS=${CLIENT_CPUS:-8-15,24-31}
DB_MEM=${DB_MEM:-16g}
PASSWORD=bench
NEO4J_PASSWORD=benchbench1 # Neo4j refuses a password under eight characters.
ARGS=(--people "${PEOPLE:-1000000}" --products "${PRODUCTS:-10000}" --concurrency "${CONCURRENCY:-1,8}"
  --warm "${WARM:-5}" --secs "${SECS:-20}" --runs "${RUNS:-3}")
[[ -n ${ONLY:-} ]] && ARGS+=(--only "$ONLY")

# Pinned by digest; the tag is for the reader.
SOLIDB_IMAGE=${SOLIDB_IMAGE:-ghcr.io/solisoft/solidb:v2.10.3}
NEO4J_IMAGE=docker.io/library/neo4j@sha256:4a2fd4f41ce384aa7d6a74b399f22c202e154f3966119c28a18ed9a500904f5e     # community
MEMGRAPH_IMAGE=docker.io/memgraph/memgraph@sha256:a732421217951066e42f5749111bc3d0c7b51b12c756be04850d4dafa84cc05e # latest, 2026-11-14
ARANGO_IMAGE=docker.io/arangodb/arangodb@sha256:258323b83ed5cd5bacc6e2d7574777db5a8a61ec75f663e4d690824c3301c7c2 # 3.12.4-3
POSTGRES_IMAGE=docker.io/library/postgres@sha256:e3d24f6906d0760bf4da0dae872415391ededefad23fd471c47dc53b0ddf3c42  # 18.6

# PostgreSQL is not a graph database and is left out of `all` (decided
# 2026-11-14); its configuration stays available by name.
ALL=(solidb neo4j memgraph arango)
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
  local data=$ROOT/data/graph-$cfg
  local engine port image settle=5 extra=() run=()
  $SUDO rm -rf "$data"
  mkdir -p "$data"
  # shellcheck disable=SC2206
  local base=($DOCKER run -d --name "$NAME" --network host --cpuset-cpus "$DB_CPUS"
    --memory "$DB_MEM" --memory-swap "$DB_MEM")
  case $cfg in
    solidb)
      engine=solidb port=6800 image=$SOLIDB_IMAGE
      # What a traversal keeps between queries, per map (the server's
      # default is 256): room for the whole graph, as Neo4j's page cache
      # and Memgraph's memory limit are given below. The write buffer (64 MB
      # by default) is what the load is sensitive to: a larger one flushes
      # and compacts less while ten million edges come in.
      run=("${base[@]}" -e SOLIDB_ADMIN_PASSWORD=$PASSWORD -e SOLIDB_PORT=$port -e SOLIDB_ENGINE=solistore
        -e SOLIDB_GRAPH_CACHE_MB="${SOLIDB_GRAPH_CACHE_MB:-2048}"
        -e SOLIDB_WRITE_BUFFER_SIZE="${SOLIDB_WRITE_BUFFER_SIZE:-2GB}" -v "$data:/data:Z,U" "$image")
      ;;
    neo4j)
      engine=bolt port=7687 image=$NEO4J_IMAGE settle=15
      extra=(--flavor neo4j --password "$NEO4J_PASSWORD")
      # The page cache and the heap share the container's 16 GB.
      run=("${base[@]}" -e NEO4J_AUTH=neo4j/$NEO4J_PASSWORD -e NEO4J_server_memory_heap_initial__size=4g
        -e NEO4J_server_memory_heap_max__size=4g -e NEO4J_server_memory_pagecache_size=8g
        -v "$data:/data:Z,U" "$image")
      ;;
    memgraph)
      engine=bolt port=7687 image=$MEMGRAPH_IMAGE
      extra=(--flavor memgraph)
      run=("${base[@]}" "$image" --memory-limit=14000 --storage-snapshot-interval-sec=0
        --storage-wal-enabled=false --log-level=WARNING)
      ;;
    arango)
      engine=arango port=8529 image=$ARANGO_IMAGE
      run=("${base[@]}" -e ARANGO_ROOT_PASSWORD=$PASSWORD -e ARANGODB_OVERRIDE_DETECTED_TOTAL_MEMORY=16G
        -e ARANGODB_OVERRIDE_DETECTED_NUMBER_OF_CORES=8 -v "$data:/var/lib/arangodb3:Z" "$image")
      ;;
    postgres)
      engine=postgres port=5432 image=$POSTGRES_IMAGE
      run=("${base[@]}" --shm-size 1g -e POSTGRES_PASSWORD=$PASSWORD -v "$data:/var/lib/postgresql:Z"
        "$image" -c shared_buffers=4GB -c effective_cache_size=12GB -c max_connections=200
        -c synchronous_commit=off)
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
    "${extra[@]}" "${ARGS[@]}" ${cgroup:+--cgroup "$cgroup"} --out "$OUT/$cfg.json" \
    2>&1 | tee "$OUT/$cfg.log" || status=$?
  $SUDO du -sb "$data" 2>/dev/null | cut -f1 >"$OUT/$cfg.disk_bytes" || true
  $DOCKER logs "$NAME" >"$OUT/$cfg.server.log" 2>&1 || true
  $DOCKER rm -f "$NAME" >/dev/null
  $SUDO rm -rf "$data"
  return $status
}

[ $# -gt 0 ] || { sed -n '2,20p' "$0"; exit 1; }
configs=("$@")
[ "$1" = all ] && configs=("${ALL[@]}")
failed=()
for cfg in "${configs[@]}"; do
  run_one "$cfg" || failed+=("$cfg")
done
echo "results in $OUT"
[ ${#failed[@]} -eq 0 ] || { echo "failed: ${failed[*]}" >&2; exit 1; }
