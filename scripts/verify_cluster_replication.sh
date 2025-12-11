#!/bin/bash
set -e

# Cleanup
pkill -f "solidb" || true
rm -rf ./data_node1 ./data_node2

# Build
cargo build

# Start Node 1 (Bootstrap)
echo "Starting Node 1..."
SOLIDB_ADMIN_PASSWORD=admin nohup ./target/debug/solidb \
    --port 6745 \
    --replication-port 6746 \
    --data-dir ./data_node1 \
    --node-id "node1" \
    > node1.log 2>&1 &
PID1=$!

sleep 2

# Start Node 2 (Join Node 1)
echo "Starting Node 2..."
SOLIDB_ADMIN_PASSWORD=admin nohup ./target/debug/solidb \
    --port 6747 \
    --replication-port 6748 \
    --data-dir ./data_node2 \
    --node-id "node2" \
    --peer "127.0.0.1:6746" \
    > node2.log 2>&1 &
PID2=$!

function cleanup {
    echo "Stopping nodes..."
    kill $PID1 || true
    kill $PID2 || true
}
trap cleanup EXIT

echo "Waiting for nodes to mesh..."
sleep 5

AUTH="Authorization: Basic $(echo -n 'admin:admin' | base64)"

# Check Cluster Status on Node 1
echo "Node 1 Status:"
curl -s -H "$AUTH" http://localhost:6745/_api/cluster/status | jq .

# Check Cluster Status on Node 2
echo "Node 2 Status:"
curl -s -H "$AUTH" http://localhost:6747/_api/cluster/status | jq .

# Create Collection on Node 1
echo "Creating collection 'repl_test' on Node 1..."
curl -X POST -H "$AUTH" -H "Content-Type: application/json" \
     -d '{"name": "repl_test"}' \
     http://localhost:6745/_api/database/_system/collections | jq .

echo "Waiting for replication..."
sleep 2

# Check Collection on Node 2 (List collections)
echo "Checking collection 'repl_test' on Node 2..."
COLLECTIONS=$(curl -s -H "$AUTH" http://localhost:6747/_api/database/_system/collections)
FOUND=$(echo "$COLLECTIONS" | jq '.result[] | select(.name == "repl_test") | .name')

if [ "$FOUND" == "\"repl_test\"" ]; then
    echo "SUCCESS: Collection replicated to Node 2!"
else
    echo "FAILURE: Collection not found on Node 2"
    echo "Response: $COLLECTIONS"
    echo "Node 1 Logs:"
    cat node1.log
    echo "Node 2 Logs:"
    cat node2.log
    exit 1
fi
