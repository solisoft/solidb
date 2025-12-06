#!/bin/bash
# Cluster Healing Test: Node recovery and data synchronization
# Tests that when a node goes down and comes back, it syncs missing data

set -e

# Configuration
PORTS=(6745 6755 6765)
REPL_PORTS=(6746 6756 6766)
DATA_DIRS=("/tmp/solidb-heal-node-1" "/tmp/solidb-heal-node-2" "/tmp/solidb-heal-node-3")
PIDS=()

cleanup() {
    echo ""
    echo "📦 Cleaning up..."
    for pid in "${PIDS[@]}"; do
        if kill -0 "$pid" 2>/dev/null; then
            kill "$pid" 2>/dev/null || true
        fi
    done
    for dir in "${DATA_DIRS[@]}"; do
        rm -rf "$dir"
    done
    echo "✓ Cleanup complete"
}

trap cleanup EXIT

echo "╔═══════════════════════════════════════════════════════════════════════╗"
echo "║                    CLUSTER HEALING TEST                               ║"
echo "╚═══════════════════════════════════════════════════════════════════════╝"
echo ""

# Build release binary
echo "🔨 Building release binary..."
cargo build --release --quiet 2>/dev/null

# Clean up previous test data
for dir in "${DATA_DIRS[@]}"; do
    rm -rf "$dir"
done

# Start 3 nodes
echo "🚀 Starting 3-node cluster..."
for i in {0..2}; do
    PORT=${PORTS[$i]}
    REPL_PORT=${REPL_PORTS[$i]}
    DATA_DIR=${DATA_DIRS[$i]}
    NODE_ID="node-$((i+1))"
    
    PEER_ARGS=""
    for j in {0..2}; do
        if [ $i -ne $j ]; then
            PEER_ARGS="$PEER_ARGS --peer localhost:${REPL_PORTS[$j]}"
        fi
    done
    
    echo "  ├─ Starting $NODE_ID on port $PORT..."
    ./target/release/solidb \
        --port "$PORT" \
        --replication-port "$REPL_PORT" \
        --data-dir "$DATA_DIR" \
        --node-id "$NODE_ID" \
        $PEER_ARGS \
        > /tmp/solidb-$NODE_ID.log 2>&1 &
    PIDS+=($!)
done
echo "  └─ All nodes started"
echo ""

# Wait for nodes to be ready
echo "⏳ Waiting for nodes to be ready..."
sleep 3

for PORT in "${PORTS[@]}"; do
    for attempt in {1..10}; do
        if curl -s "http://localhost:$PORT/_api/health" > /dev/null 2>&1; then
            break
        fi
        sleep 0.5
    done
done
echo "  └─ All nodes ready"
echo ""

# Use first node as coordinator
COORD_URL="http://localhost:${PORTS[0]}"
DB_NAME="test_heal"
COLL_NAME="docs"

# Create database and collection
echo "📂 Creating database and collection..."
curl -s -X POST "$COORD_URL/_api/database" \
    -H "Content-Type: application/json" \
    -d "{\"name\": \"$DB_NAME\"}" > /dev/null

curl -s -X POST "$COORD_URL/_api/database/$DB_NAME/collection" \
    -H "Content-Type: application/json" \
    -d "{\"name\": \"$COLL_NAME\"}" > /dev/null

sleep 2

# Phase 1: Insert initial data on all nodes
echo ""
echo "═══════════════════════════════════════════════════════════════════════"
echo "Phase 1: Insert 10 documents while all nodes are up"
echo "═══════════════════════════════════════════════════════════════════════"

for i in {1..10}; do
    curl -s -X POST "$COORD_URL/_api/database/$DB_NAME/document/$COLL_NAME" \
        -H "Content-Type: application/json" \
        -d "{\"_key\": \"doc-$i\", \"phase\": 1, \"value\": $i}" > /dev/null
done
echo "  ├─ Inserted 10 documents (phase 1)"
sleep 2

# Check document counts on all nodes
echo "  └─ Document counts after phase 1:"
for i in {0..2}; do
    PORT=${PORTS[$i]}
    NODE_ID="node-$((i+1))"
    COUNT=$(curl -s "http://localhost:$PORT/_api/database/$DB_NAME/cursor" \
        -H "Content-Type: application/json" \
        -d "{\"query\": \"RETURN COLLECTION_COUNT('$COLL_NAME')\"}" 2>/dev/null | jq -r '.result[0] // 0')
    printf "      %s: %s documents\n" "$NODE_ID" "$COUNT"
done
echo ""

# Phase 2: Stop node-3, insert more data
echo "═══════════════════════════════════════════════════════════════════════"
echo "Phase 2: Stop node-3, insert 10 more documents"
echo "═══════════════════════════════════════════════════════════════════════"

echo "  ├─ Stopping node-3..."
kill ${PIDS[2]} 2>/dev/null || true
unset PIDS[2]
sleep 1
echo "  ├─ node-3 stopped"

# Insert 10 more documents while node-3 is down
for i in {11..20}; do
    curl -s -X POST "$COORD_URL/_api/database/$DB_NAME/document/$COLL_NAME" \
        -H "Content-Type: application/json" \
        -d "{\"_key\": \"doc-$i\", \"phase\": 2, \"value\": $i}" > /dev/null
done
echo "  ├─ Inserted 10 more documents (phase 2) while node-3 is down"

sleep 1
echo "  └─ Document counts (node-3 is down):"
for i in {0..1}; do
    PORT=${PORTS[$i]}
    NODE_ID="node-$((i+1))"
    COUNT=$(curl -s "http://localhost:$PORT/_api/database/$DB_NAME/cursor" \
        -H "Content-Type: application/json" \
        -d "{\"query\": \"RETURN COLLECTION_COUNT('$COLL_NAME')\"}" 2>/dev/null | jq -r '.result[0] // 0')
    printf "      %s: %s documents\n" "$NODE_ID" "$COUNT"
done
echo ""

# Phase 3: Restart node-3 and wait for healing
echo "═══════════════════════════════════════════════════════════════════════"
echo "Phase 3: Restart node-3 and verify healing"
echo "═══════════════════════════════════════════════════════════════════════"

echo "  ├─ Starting node-3 again..."
./target/release/solidb \
    --port "${PORTS[2]}" \
    --replication-port "${REPL_PORTS[2]}" \
    --data-dir "${DATA_DIRS[2]}" \
    --node-id "node-3" \
    --peer "localhost:${REPL_PORTS[0]}" \
    --peer "localhost:${REPL_PORTS[1]}" \
    > /tmp/solidb-node-3-restart.log 2>&1 &
PIDS[2]=$!

echo "  ├─ Waiting for node-3 to sync..."
sleep 5

# Verify node-3 has healed
NODE3_COUNT=$(curl -s "http://localhost:${PORTS[2]}/_api/database/$DB_NAME/cursor" \
    -H "Content-Type: application/json" \
    -d "{\"query\": \"RETURN COLLECTION_COUNT('$COLL_NAME')\"}" 2>/dev/null | jq -r '.result[0] // 0')

echo "  └─ Document counts after healing:"
for i in {0..2}; do
    PORT=${PORTS[$i]}
    NODE_ID="node-$((i+1))"
    COUNT=$(curl -s "http://localhost:$PORT/_api/database/$DB_NAME/cursor" \
        -H "Content-Type: application/json" \
        -d "{\"query\": \"RETURN COLLECTION_COUNT('$COLL_NAME')\"}" 2>/dev/null | jq -r '.result[0] // 0')
    printf "      %s: %s documents\n" "$NODE_ID" "$COUNT"
done
echo ""

# Verify healing
EXPECTED=20
if [ "$NODE3_COUNT" -ge "$EXPECTED" ]; then
    echo "╔═══════════════════════════════════════════════════════════════════════╗"
    echo "║                          ✅ HEALING PASSED                            ║"
    echo "╠═══════════════════════════════════════════════════════════════════════╣"
    printf "║  Expected: %-3d documents on node-3                                   ║\n" "$EXPECTED"
    printf "║  Actual:   %-3d documents on node-3                                   ║\n" "$NODE3_COUNT"
    echo "║  ✓ Node-3 successfully synced after restart                           ║"
    echo "╚═══════════════════════════════════════════════════════════════════════╝"
    exit 0
else
    echo "╔═══════════════════════════════════════════════════════════════════════╗"
    echo "║                          ❌ HEALING FAILED                            ║"
    echo "╠═══════════════════════════════════════════════════════════════════════╣"
    printf "║  Expected: %-3d documents on node-3                                   ║\n" "$EXPECTED"
    printf "║  Actual:   %-3d documents on node-3                                   ║\n" "$NODE3_COUNT"
    echo "║  ⚠️  Node-3 did not sync all documents after restart                   ║"
    echo "╚═══════════════════════════════════════════════════════════════════════╝"
    exit 1
fi
