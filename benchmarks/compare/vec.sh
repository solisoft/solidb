#!/usr/bin/env bash
# The vector benchmark: the same vectors and the same nearest-neighbour
# searches against SoliDB, PostgreSQL + pgvector and LanceDB (card #979).
#
#   ./vec.sh all                     every configuration, one after another
#   ./vec.sh solidb lancedb          only those
#   N=20000 DIM=64 QUERIES=200 SECS=3 RUNS=1 ./vec.sh all     smoke pass
#
# Configurations:
#   solidb     a SoliDB binary (SOLIDB_BIN), solistore; HNSW index
#              on the documents' `emb` field, VECTOR_SEARCH over the driver.
#              SOLIDB_ARGS adds options to the server's command line.
#              QUANT=scalar|binary|pq quantizes the index; the results are
#              then solidb-$QUANT.* instead of solidb.*.
#   pgvector   PostgreSQL + pgvector in a container on DB_CPUS and DB_MEM;
#              HNSW (vector_cosine_ops) built after the load.
#   lancedb    LanceDB embedded, in the client's process (vec_lance.py in
#              LANCE_PYTHON's environment), held to the servers' limits, since
#              that process does the work a server does for the others. No network.
#
# SoliDB and LanceDB run in a systemd scope with the container's limits
# (AllowedCPUs=DB_CPUS, MemoryMax=DB_MEM, no swap): the same cores and
# memory as pgvector's container, without building an image.
#
# Every engine gets the same HNSW settings (M=16, ef_construction=200) and is
# measured at the same ef values: recall@10 against the exact answers the
# harness computes, then requests/s and latency. A faster search with a lower
# recall is not a win; read the two together.
set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
DOCKER=${DOCKER:-sudo podman}
BIN=${BIN:-$HERE/target/release/solidb-vec}
SOLIDB_BIN=${SOLIDB_BIN:-$HERE/../../target/release/solidb}
LANCE_PYTHON=${LANCE_PYTHON:-python3}
ROOT=${BENCH_ROOT:-$HOME/bench}
OUT=${OUT:-$ROOT/results/vec-$(date +%F)}
DB_CPUS=${DB_CPUS:-0-3,16-19}
CLIENT_CPUS=${CLIENT_CPUS:-8-15,24-31}
DB_MEM=${DB_MEM:-16g}
PASSWORD=bench
# Pinned by digest; the tag is for the reader.
PGVECTOR_IMAGE=${PGVECTOR_IMAGE:-docker.io/pgvector/pgvector@sha256:0a547a6067fdea4eec4cfb0887c4ddae2e91b1932cecc071541c14ee318312ce} # pg17, 2026-11-18
ARGS=(--n "${N:-1000000}" --dim "${DIM:-128}" --queries "${QUERIES:-500}" --k 10
  --ef "${EF:-40,100,200}" --concurrency "${CONCURRENCY:-1,8}"
  --warm "${WARM:-3}" --secs "${SECS:-10}" --runs "${RUNS:-3}")
ALL=(solidb pgvector lancedb)
NAME=bench-vec
mkdir -p "$OUT"

wait_port() {
  local i
  for i in $(seq 1 600); do
    (exec 3<>"/dev/tcp/127.0.0.1/$1") 2>/dev/null && { sleep "${2:-2}"; return 0; }
    sleep 0.5
  done
  echo "port $1 never opened" >&2
  return 1
}

# Run a command with the database's limits: DB_CPUS, DB_MEM, no swap.
limited() {
  sudo systemd-run --scope --quiet -p AllowedCPUs="$DB_CPUS" -p MemoryMax="${DB_MEM^^}" \
    -p MemorySwapMax=0 --uid="$(id -u)" --gid="$(id -g)" "$@"
}

run_one() {
  local cfg=$1 data=$ROOT/data/vec-$cfg status=0
  $SUDO_RM "$data"
  mkdir -p "$data"
  case $cfg in
    solidb)
      local tag=solidb${QUANT:+-$QUANT} quant=()
      [ -n "${QUANT:-}" ] && quant=(--quantization "$QUANT")
      limited env SOLIDB_ADMIN_PASSWORD=$PASSWORD SOLIDB_ENGINE=solistore \
        "$SOLIDB_BIN" --host 127.0.0.1 --port 6800 --data-dir "$data" --no-lua ${SOLIDB_ARGS:-} >"$OUT/$tag.server.log" 2>&1 &
      local scope=$!
      wait_port 6800
      # $! is sudo's; the server is the process listening on the port.
      local server
      server=$(ss -Hltnp 'sport = :6800' | grep -o 'pid=[0-9]*' | head -1 | cut -d= -f2)
      taskset -c "$CLIENT_CPUS" "$BIN" --engine solidb --addr 127.0.0.1:6800 --password $PASSWORD \
        "${ARGS[@]}" "${quant[@]}" --out "$OUT/$tag.json" 2>&1 | tee "$OUT/$tag.log" || status=$?
      echo "rss_kb $(ps -o rss= -p "$server")" >"$OUT/$tag.memory"
      kill "$server"
      wait $scope 2>/dev/null || true
      ;;
    pgvector)
      $DOCKER rm -f $NAME >/dev/null 2>&1 || true
      $DOCKER run -d --name $NAME --network host --cpuset-cpus "$DB_CPUS" --memory "$DB_MEM" \
        --memory-swap "$DB_MEM" --shm-size 4g -e POSTGRES_PASSWORD=$PASSWORD \
        -v "$data:/var/lib/postgresql/data:Z" "$PGVECTOR_IMAGE" \
        -c shared_buffers=4GB -c maintenance_work_mem=4GB -c max_parallel_maintenance_workers=7 \
        -c max_connections=200 >/dev/null
      wait_port 5432 10
      taskset -c "$CLIENT_CPUS" "$BIN" --engine pgvector --addr 127.0.0.1:5432 --password $PASSWORD \
        "${ARGS[@]}" --out "$OUT/pgvector.json" 2>&1 | tee "$OUT/pgvector.log" || status=$?
      $DOCKER stats --no-stream --format '{{.MemUsage}}' $NAME >"$OUT/pgvector.memory" 2>/dev/null || true
      $DOCKER logs $NAME >"$OUT/pgvector.server.log" 2>&1 || true
      $DOCKER rm -f $NAME >/dev/null
      ;;
    lancedb)
      "$BIN" --engine dump --addr - "${ARGS[@]}" --out "$data/dump" 2>&1 | tail -1
      local ef conc
      ef=$(sed -n 's/.*--ef \([^ ]*\).*/\1/p' <<<"${ARGS[*]}")
      conc=$(sed -n 's/.*--concurrency \([^ ]*\).*/\1/p' <<<"${ARGS[*]}")
      limited "$LANCE_PYTHON" "$HERE/vec_lance.py" "$data/dump" --ef "$ef" --concurrency "$conc" \
        --warm "${WARM:-3}" --secs "${SECS:-10}" --runs "${RUNS:-3}" --table-dir "$data/table" \
        --out "$OUT/lancedb.json" 2>&1 | tee "$OUT/lancedb.log" || status=$?
      ;;
    *)
      echo "unknown configuration $cfg" >&2
      return 1
      ;;
  esac
  $SUDO_RM "$data"
  return $status
}

SUDO_RM="sudo rm -rf"
[ $# -gt 0 ] || { sed -n '2,22p' "$0"; exit 1; }
configs=("$@")
[ "$1" = all ] && configs=("${ALL[@]}")
failed=()
for cfg in "${configs[@]}"; do
  run_one "$cfg" || failed+=("$cfg")
done
echo "results in $OUT"
[ ${#failed[@]} -eq 0 ] || { echo "failed: ${failed[*]}" >&2; exit 1; }
