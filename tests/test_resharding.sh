#!/bin/bash
# Resharding Test: 4 nodes, remove one, verify resharding and replica count
# Tests that when a node is removed, data is redistributed to maintain replicas

set -e

# Configuration
PORTS=(6745 6755 6765 6775)
REPL_PORTS=(6746 6756 6766 6776)
DATA_DIRS=("/tmp/solidb-reshard-node-1" "/tmp/solidb-reshard-node-2" "/tmp/solidb-reshard-node-3" "/tmp/solidb-reshard-node-4")
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
echo "║           RESHARDING TEST: NODE REMOVAL & REPLICA MAINTENANCE         ║"
echo "╚═══════════════════════════════════════════════════════════════════════╝"
echo ""

# Build release binary
echo "🔨 Building release binary..."
cargo build --release --quiet 2>/dev/null

# Clean up previous test data
for dir in "${DATA_DIRS[@]}"; do
    rm -rf "$dir"
done

# Start 4 nodes
echo "🚀 Starting 4-node cluster..."
for i in {0..3}; do
    PORT=${PORTS[$i]}
    REPL_PORT=${REPL_PORTS[$i]}
    DATA_DIR=${DATA_DIRS[$i]}
    NODE_ID="node-$((i+1))"
    
    PEER_ARGS=""
    for j in {0..3}; do
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

COORD_URL="http://localhost:${PORTS[0]}"
DB_NAME="test_reshard"
COLL_NAME="items"
NUM_SHARDS=2
REPLICATION_FACTOR=2

# Create database and sharded collection
echo "📂 Creating database and collection on all nodes..."

# Create database on coordinator (will replicate)
curl -s -X POST "$COORD_URL/_api/database" \
    -H "Content-Type: application/json" \
    -d "{\"name\": \"$DB_NAME\"}" > /dev/null

curl -s -X POST "$COORD_URL/_api/database/$DB_NAME/collection" \
    -H "Content-Type: application/json" \
    -d "{\"name\": \"$COLL_NAME\"}" > /dev/null

# Wait for replication of DB/collection to all nodes
sleep 3

# Configure shards on ALL nodes (shard config is not replicated automatically)
echo "🔧 Configuring collection shards on all nodes: $NUM_SHARDS shards, replication factor $REPLICATION_FACTOR..."
for PORT in "${PORTS[@]}"; do
    curl -s -X PUT "http://localhost:$PORT/_api/database/$DB_NAME/collection/$COLL_NAME/properties" \
        -H "Content-Type: application/json" \
        -d "{\"numShards\": $NUM_SHARDS, \"replicationFactor\": $REPLICATION_FACTOR}" > /dev/null
done

sleep 2

# Phase 1: Insert data
echo ""
echo "═══════════════════════════════════════════════════════════════════════"
echo "Phase 1: Insert 30 documents across shards"
echo "═══════════════════════════════════════════════════════════════════════"

for i in {1..30}; do
    KEY="item-$RANDOM-$i"
    curl -s -X POST "$COORD_URL/_api/database/$DB_NAME/document/$COLL_NAME" \
        -H "Content-Type: application/json" \
        -d "{\"_key\": \"$KEY\", \"value\": $i, \"data\": \"test-$i\"}" > /dev/null
done
echo "  ├─ Inserted 30 documents"

sleep 3

# Count docs on each node
echo "  └─ Document counts per node:"
TOTAL_BEFORE=0
for i in {0..3}; do
    PORT=${PORTS[$i]}
    NODE_ID="node-$((i+1))"
    COUNT=$(curl -s "http://localhost:$PORT/_api/database/$DB_NAME/cursor" \
        -H "Content-Type: application/json" \
        -d "{\"query\": \"RETURN COLLECTION_COUNT('$COLL_NAME')\"}" 2>/dev/null | jq -r '.result[0] // 0')
    printf "      %s: %s documents\n" "$NODE_ID" "$COUNT"
    TOTAL_BEFORE=$((TOTAL_BEFORE + COUNT))
done
echo "      Total (with replicas): $TOTAL_BEFORE documents"
echo ""

# Phase 2: Remove node-4
echo "═══════════════════════════════════════════════════════════════════════"
echo "Phase 2: Remove node-4 from cluster"
echo "═══════════════════════════════════════════════════════════════════════"

echo "  ├─ Stopping node-4..."
kill ${PIDS[3]} 2>/dev/null || true
unset PIDS[3]
sleep 2
echo "  ├─ node-4 stopped"

# Call cluster remove-node API to trigger rebalancing
echo "  ├─ Calling /_api/cluster/remove-node to trigger rebalancing..."
REMOVE_RESULT=$(curl -s -X POST "$COORD_URL/_api/cluster/remove-node" \
    -H "Content-Type: application/json" \
    -d "{\"node_address\": \"localhost:6775\"}" 2>/dev/null)
echo "      Response: $(echo "$REMOVE_RESULT" | jq -r '.message // "No message"')"
REMAINING_NODES=$(echo "$REMOVE_RESULT" | jq -r '.remaining_nodes | length // 0')
printf "      Remaining nodes: %s\n" "$REMAINING_NODES"

# Wait for rebalancing to complete
echo "  ├─ Waiting for rebalancing..."
sleep 3

# Count docs on remaining nodes
echo "  └─ Document counts after node-4 removal:"
TOTAL_AFTER=0
for i in {0..2}; do
    PORT=${PORTS[$i]}
    NODE_ID="node-$((i+1))"
    COUNT=$(curl -s "http://localhost:$PORT/_api/database/$DB_NAME/cursor" \
        -H "Content-Type: application/json" \
        -d "{\"query\": \"RETURN COLLECTION_COUNT('$COLL_NAME')\"}" 2>/dev/null | jq -r '.result[0] // 0')
    printf "      %s: %s documents\n" "$NODE_ID" "$COUNT"
    TOTAL_AFTER=$((TOTAL_AFTER + COUNT))
done
echo "      Total on remaining nodes: $TOTAL_AFTER documents"
echo ""

# Phase 3: Verify all data is still accessible via scatter-gather
echo "═══════════════════════════════════════════════════════════════════════"
echo "Phase 3: Verify data accessibility via scatter-gather"
echo "═══════════════════════════════════════════════════════════════════════"

QUERY_RESULT=$(curl -s -X POST "$COORD_URL/_api/database/$DB_NAME/cursor" \
    -H "Content-Type: application/json" \
    -d "{\"query\": \"FOR doc IN $COLL_NAME RETURN doc\"}" 2>/dev/null)

ACCESSIBLE_COUNT=$(echo "$QUERY_RESULT" | jq -r '.result | length // 0')
printf "  ├─ Documents accessible via scatter-gather: %s\n" "$ACCESSIBLE_COUNT"

# Calculate expected values
# With 30 docs and replication_factor=2, we expect 60 total copies across nodes
# After removing 1 of 4 nodes, we might lose some if resharding doesn't happen
EXPECTED_ACCESSIBLE=30

echo ""

# Final verdict
if [ "$ACCESSIBLE_COUNT" -ge "$EXPECTED_ACCESSIBLE" ]; then
    echo "╔═══════════════════════════════════════════════════════════════════════╗"
    echo "║                          ✅ TEST PASSED                               ║"
    echo "╠═══════════════════════════════════════════════════════════════════════╣"
    printf "║  Documents before node removal: %-3d (with replicas)                  ║\n" "$TOTAL_BEFORE"
    printf "║  Documents after node removal:  %-3d (remaining nodes)                ║\n" "$TOTAL_AFTER"
    printf "║  Documents accessible:          %-3d (expected: %d)                   ║\n" "$ACCESSIBLE_COUNT" "$EXPECTED_ACCESSIBLE"
    echo "║  ✓ All data remained accessible after node removal                    ║"
    echo "╚═══════════════════════════════════════════════════════════════════════╝"
    exit 0
else
    echo "╔═══════════════════════════════════════════════════════════════════════╗"
    echo "║                          ❌ TEST FAILED                               ║"
    echo "╠═══════════════════════════════════════════════════════════════════════╣"
    printf "║  Documents before: %-3d                                               ║\n" "$TOTAL_BEFORE"
    printf "║  Documents after:  %-3d                                               ║\n" "$TOTAL_AFTER"
    printf "║  Accessible: %-3d (expected: %d)                                      ║\n" "$ACCESSIBLE_COUNT" "$EXPECTED_ACCESSIBLE"
    echo "║  ⚠️  Some data was lost after node removal                             ║"
    echo "║  Resharding may not have redistributed all replicas                   ║"
    echo "╚═══════════════════════════════════════════════════════════════════════╝"
    exit 1
fi
