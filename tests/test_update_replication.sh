#!/bin/bash
# Update Replication Test: Verify updates sync when a node fails and recovers
# Tests that document updates made while a node is down get synced on recovery

set -e

# Configuration
PORTS=(6745 6755 6765)
REPL_PORTS=(6746 6756 6766)
DATA_DIRS=("/tmp/solidb-update-node-1" "/tmp/solidb-update-node-2" "/tmp/solidb-update-node-3")
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
echo "║              UPDATE REPLICATION DURING NODE FAILURE                   ║"
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

COORD_URL="http://localhost:${PORTS[0]}"
DB_NAME="test_updates"
COLL_NAME="users"

# Create database and collection
echo "📂 Creating database and collection..."
curl -s -X POST "$COORD_URL/_api/database" \
    -H "Content-Type: application/json" \
    -d "{\"name\": \"$DB_NAME\"}" > /dev/null

curl -s -X POST "$COORD_URL/_api/database/$DB_NAME/collection" \
    -H "Content-Type: application/json" \
    -d "{\"name\": \"$COLL_NAME\"}" > /dev/null

sleep 2

# Phase 1: Insert initial documents
echo ""
echo "═══════════════════════════════════════════════════════════════════════"
echo "Phase 1: Insert 5 documents with status='active'"
echo "═══════════════════════════════════════════════════════════════════════"

for i in {1..5}; do
    curl -s -X POST "$COORD_URL/_api/database/$DB_NAME/document/$COLL_NAME" \
        -H "Content-Type: application/json" \
        -d "{\"_key\": \"user-$i\", \"name\": \"User $i\", \"status\": \"active\", \"version\": 1}" > /dev/null
done
echo "  ├─ Inserted 5 documents"
sleep 2

# Check values on all nodes
echo "  └─ Checking user-1 status on all nodes:"
for i in {0..2}; do
    PORT=${PORTS[$i]}
    NODE_ID="node-$((i+1))"
    STATUS=$(curl -s "http://localhost:$PORT/_api/database/$DB_NAME/document/$COLL_NAME/user-1" 2>/dev/null | jq -r '.status // "N/A"')
    VERSION=$(curl -s "http://localhost:$PORT/_api/database/$DB_NAME/document/$COLL_NAME/user-1" 2>/dev/null | jq -r '.version // "N/A"')
    printf "      %s: status=%s, version=%s\n" "$NODE_ID" "$STATUS" "$VERSION"
done
echo ""

# Phase 2: Stop node-3, update documents
echo "═══════════════════════════════════════════════════════════════════════"
echo "Phase 2: Stop node-3, update documents"
echo "═══════════════════════════════════════════════════════════════════════"

echo "  ├─ Stopping node-3..."
kill ${PIDS[2]} 2>/dev/null || true
unset PIDS[2]
sleep 1
echo "  ├─ node-3 stopped"

# Update all documents while node-3 is down
echo "  ├─ Updating 5 documents (status='inactive', version=2) via node-1..."
for i in {1..5}; do
    curl -s -X PUT "$COORD_URL/_api/database/$DB_NAME/document/$COLL_NAME/user-$i" \
        -H "Content-Type: application/json" \
        -d "{\"status\": \"inactive\", \"version\": 2}" > /dev/null
done
echo "  ├─ All documents updated"

sleep 1
echo "  └─ Checking user-1 on running nodes:"
for i in {0..1}; do
    PORT=${PORTS[$i]}
    NODE_ID="node-$((i+1))"
    STATUS=$(curl -s "http://localhost:$PORT/_api/database/$DB_NAME/document/$COLL_NAME/user-1" 2>/dev/null | jq -r '.status // "N/A"')
    VERSION=$(curl -s "http://localhost:$PORT/_api/database/$DB_NAME/document/$COLL_NAME/user-1" 2>/dev/null | jq -r '.version // "N/A"')
    printf "      %s: status=%s, version=%s\n" "$NODE_ID" "$STATUS" "$VERSION"
done
echo ""

# Phase 3: Restart node-3 and verify updates synced
echo "═══════════════════════════════════════════════════════════════════════"
echo "Phase 3: Restart node-3 and verify updates synced"
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
sleep 10

# Check if node-3 has the updated values
NODE3_STATUS=$(curl -s "http://localhost:${PORTS[2]}/_api/database/$DB_NAME/document/$COLL_NAME/user-1" 2>/dev/null | jq -r '.status // "N/A"')
NODE3_VERSION=$(curl -s "http://localhost:${PORTS[2]}/_api/database/$DB_NAME/document/$COLL_NAME/user-1" 2>/dev/null | jq -r '.version // "N/A"')

echo "  └─ Checking user-1 on all nodes after healing:"
for i in {0..2}; do
    PORT=${PORTS[$i]}
    NODE_ID="node-$((i+1))"
    STATUS=$(curl -s "http://localhost:$PORT/_api/database/$DB_NAME/document/$COLL_NAME/user-1" 2>/dev/null | jq -r '.status // "N/A"')
    VERSION=$(curl -s "http://localhost:$PORT/_api/database/$DB_NAME/document/$COLL_NAME/user-1" 2>/dev/null | jq -r '.version // "N/A"')
    printf "      %s: status=%s, version=%s\n" "$NODE_ID" "$STATUS" "$VERSION"
done
echo ""

# Verify healing
if [ "$NODE3_STATUS" = "inactive" ] && [ "$NODE3_VERSION" = "2" ]; then
    echo "╔═══════════════════════════════════════════════════════════════════════╗"
    echo "║                    ✅ UPDATE REPLICATION PASSED                       ║"
    echo "╠═══════════════════════════════════════════════════════════════════════╣"
    echo "║  ✓ Node-3 received updates made while it was offline                  ║"
    printf "║  ✓ user-1: status=%s, version=%s                               ║\n" "$NODE3_STATUS" "$NODE3_VERSION"
    echo "╚═══════════════════════════════════════════════════════════════════════╝"
    exit 0
else
    echo "╔═══════════════════════════════════════════════════════════════════════╗"
    echo "║                    ❌ UPDATE REPLICATION FAILED                       ║"
    echo "╠═══════════════════════════════════════════════════════════════════════╣"
    echo "║  ⚠️  Node-3 did not receive updates made while it was offline          ║"
    printf "║  Expected: status=inactive, version=2                                ║\n"
    printf "║  Got:      status=%s, version=%s                                  ║\n" "$NODE3_STATUS" "$NODE3_VERSION"
    echo "╚═══════════════════════════════════════════════════════════════════════╝"
    exit 1
fi
