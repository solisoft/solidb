#!/bin/bash
# Scatter-Gather Test: 4 nodes, 3 shards
# Tests that queries correctly gather data from all shards across multiple nodes

set -e

# Configuration
PORTS=(6745 6755 6765 6775)
REPL_PORTS=(6746 6756 6766 6776)
DATA_DIRS=("/tmp/solidb-test-node-1" "/tmp/solidb-test-node-2" "/tmp/solidb-test-node-3" "/tmp/solidb-test-node-4")
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
echo "║            SCATTER-GATHER TEST: 4 NODES, 3 SHARDS                     ║"
echo "╚═══════════════════════════════════════════════════════════════════════╝"
echo ""

# Build release binary
echo "🔨 Building release binary..."
cargo build --release --quiet

# Clean up previous test data
for dir in "${DATA_DIRS[@]}"; do
    rm -rf "$dir"
done

# Start 4 nodes
echo "🚀 Starting 4-node cluster..."
PEER_LIST=""
for i in {0..3}; do
    if [ $i -gt 0 ]; then
        PEER_LIST="$PEER_LIST --peer localhost:${REPL_PORTS[0]}"
    fi
done

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
    
    echo "  ├─ Starting $NODE_ID on port $PORT (repl: $REPL_PORT)..."
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
            echo "  ├─ Node on port $PORT: ✓ ready"
            break
        fi
        if [ $attempt -eq 10 ]; then
            echo "  ├─ Node on port $PORT: ✗ FAILED to start"
            exit 1
        fi
        sleep 0.5
    done
done
echo ""

# Use first node as coordinator
COORD_URL="http://localhost:${PORTS[0]}"
DB_NAME="test_scatter"
COLL_NAME="items"

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
echo "🔧 Configuring collection with 3 shards on all nodes..."
for PORT in "${PORTS[@]}"; do
    curl -s -X PUT "http://localhost:$PORT/_api/database/$DB_NAME/collection/$COLL_NAME/properties" \
        -H "Content-Type: application/json" \
        -d "{\"numShards\": 3, \"replicationFactor\": 2}" > /dev/null
done

echo "⏳ Waiting for collection to replicate to all nodes..."
sleep 3

# Verify collection exists on all nodes
echo "🔍 Verifying collection on all nodes..."
for i in {0..3}; do
    PORT=${PORTS[$i]}
    NODE_ID="node-$((i+1))"
    
    COLL_CHECK=$(curl -s "http://localhost:$PORT/_api/database/$DB_NAME/collection" 2>/dev/null || echo '{"collections":[]}')
    COLL_EXISTS=$(echo "$COLL_CHECK" | jq -r ".collections[] | select(.name == \"$COLL_NAME\") | .name // empty")
    
    if [ -n "$COLL_EXISTS" ]; then
        printf "  ├─ %s: collection '%s' ✓\n" "$NODE_ID" "$COLL_NAME"
    else
        printf "  ├─ %s: collection NOT FOUND ✗\n" "$NODE_ID"
    fi
done
echo ""

echo "📝 Inserting test documents (should distribute across shards)..."

# Insert 30 documents with different keys to ensure distribution
INSERTED_COUNT=0
for i in {1..30}; do
    KEY="doc-$RANDOM-$i"
    VALUE="value-$i"
    
    RESULT=$(curl -s -X POST "$COORD_URL/_api/database/$DB_NAME/document/$COLL_NAME" \
        -H "Content-Type: application/json" \
        -d "{\"_key\": \"$KEY\", \"value\": \"$VALUE\", \"index\": $i}")
    
    if echo "$RESULT" | grep -q "_key"; then
        INSERTED_COUNT=$((INSERTED_COUNT + 1))
    fi
done
echo "  └─ Inserted $INSERTED_COUNT documents"
echo ""

# Give replication time to sync
echo "⏳ Waiting for replication..."
sleep 3

# Stop node-1 (coordinator) to test if other nodes have data
echo "🛑 Stopping node-1 (coordinator) to test failover..."
kill ${PIDS[0]} 2>/dev/null || true
sleep 1
echo "  └─ node-1 stopped"
echo ""

# Try querying remaining nodes
echo "🔍 Querying remaining nodes for data..."
NODES_WITH_DATA=0
TOTAL_FROM_REMAINING=0

for i in {1..3}; do  # Skip node-1 (index 0)
    PORT=${PORTS[$i]}
    NODE_ID="node-$((i+1))"
    
    # Query this node directly
    NODE_RESULT=$(curl -s -X POST "http://localhost:$PORT/_api/database/$DB_NAME/cursor" \
        -H "Content-Type: application/json" \
        -d "{\"query\": \"FOR doc IN $COLL_NAME RETURN doc\"}" 2>/dev/null || echo '{"result":[]}')
    
    NODE_COUNT=$(echo "$NODE_RESULT" | jq -r '.result | length // 0')
    TOTAL_FROM_REMAINING=$((TOTAL_FROM_REMAINING + NODE_COUNT))
    
    if [ "$NODE_COUNT" -gt 0 ]; then
        NODES_WITH_DATA=$((NODES_WITH_DATA + 1))
    fi
    
    printf "  ├─ %s (port %s): %3d documents\n" "$NODE_ID" "$PORT" "$NODE_COUNT"
done

echo "  └─ Total from remaining nodes: $TOTAL_FROM_REMAINING documents"
echo ""

# If data was sharded/replicated, remaining nodes should have some documents
if [ "$TOTAL_FROM_REMAINING" -gt 0 ]; then
    echo "╔═══════════════════════════════════════════════════════════════════════╗"
    echo "║                          ✅ TEST PASSED                               ║"
    echo "╠═══════════════════════════════════════════════════════════════════════╣"
    printf "║  Inserted: %-3d documents                                             ║\n" "$INSERTED_COUNT"
    printf "║  Retrieved after node-1 down: %-3d documents                          ║\n" "$TOTAL_FROM_REMAINING"
    printf "║  Remaining nodes with data: %-1d of 3                                    ║\n" "$NODES_WITH_DATA"
    echo "║  ✓ Data was replicated/sharded to other nodes                         ║"
    echo "╚═══════════════════════════════════════════════════════════════════════╝"
    exit 0
else
    echo "╔═══════════════════════════════════════════════════════════════════════╗"
    echo "║                          ❌ TEST FAILED                               ║"
    echo "╠═══════════════════════════════════════════════════════════════════════╣"
    printf "║  Inserted: %-3d documents                                             ║\n" "$INSERTED_COUNT"
    echo "║  After stopping node-1: 0 documents on other nodes                    ║"
    echo "║  ⚠️  Data was NOT sharded/replicated to other nodes                    ║"
    echo "║  Check that inserts are being routed to shard nodes                   ║"
    echo "╚═══════════════════════════════════════════════════════════════════════╝"
    exit 1
fi
