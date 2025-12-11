#!/bin/bash
# Test import distribution across a 3-node cluster

set -e

SCRIPT_DIR=$(dirname "$0")
cd "$SCRIPT_DIR/.."

BIN="./target/debug/solidb"

# Build first
echo "Building..."
cargo build --quiet

# Clean up any existing test data
rm -rf /tmp/import_test_data
mkdir -p /tmp/import_test_data/{n1,n2,n3}

# Kill any existing instances
pkill -f "solidb.*import_test" 2>/dev/null || true
sleep 1

echo "Starting 3-node cluster..."

# Set admin password for all nodes
export SOLIDB_ADMIN_PASSWORD="admin"

# Start node 1 (bootstrap)
$BIN --port 7001 --replication-port 8001 --data-dir /tmp/import_test_data/n1 > /tmp/import_test_data/n1.log 2>&1 &
PID1=$!
sleep 2

# Start node 2
$BIN --port 7002 --replication-port 8002 --peer 127.0.0.1:8001 --data-dir /tmp/import_test_data/n2 > /tmp/import_test_data/n2.log 2>&1 &
PID2=$!
sleep 2

# Start node 3
$BIN --port 7003 --replication-port 8003 --peer 127.0.0.1:8001 --data-dir /tmp/import_test_data/n3 > /tmp/import_test_data/n3.log 2>&1 &
PID3=$!
sleep 3

echo "Nodes started: $PID1, $PID2, $PID3"

# Create database
echo "Creating test database..."
DB_RESULT=$(curl -s -X POST "http://admin:admin@127.0.0.1:7001/_api/database" \
    -H "Content-Type: application/json" \
    -d '{"name": "testdb"}')
echo "Database result: $DB_RESULT"

sleep 2

# Create sharded collection on ALL nodes (to avoid replication delays)
echo "Creating sharded collection on all nodes..."
for port in 7001 7002 7003; do
    RESULT=$(curl -s -X POST "http://admin:admin@127.0.0.1:$port/_api/database/testdb/collection" \
        -H "Content-Type: application/json" \
        -d '{"name": "users", "numShards": 3, "replicationFactor": 2}')
    echo "Node $port: $RESULT"
done

sleep 2

# Generate test data file
echo "Generating 1000 test documents..."
rm -f /tmp/import_test_data/test.jsonl
for i in $(seq 1 1000); do
    echo "{\"_key\": \"user$i\", \"name\": \"User $i\", \"email\": \"user$i@test.com\"}" >> /tmp/import_test_data/test.jsonl
done

# Import to node 3
echo "Importing 1000 docs to node 3..."
IMPORT_RESULT=$(curl -s -X POST "http://admin:admin@127.0.0.1:7003/_api/database/testdb/collection/users/import" \
    -F "file=@/tmp/import_test_data/test.jsonl")
echo "Import result: $IMPORT_RESULT"

sleep 5

# Check data folder sizes
echo ""
echo "=== Data folder sizes ==="
du -sh /tmp/import_test_data/n1
du -sh /tmp/import_test_data/n2
du -sh /tmp/import_test_data/n3

# Check stats from each node
echo ""
echo "=== Stats from node 1 ==="
curl -s "http://admin:admin@127.0.0.1:7001/_api/database/testdb/collection/users/stats" | jq '.cluster.shards'

echo ""
echo "=== Stats from node 2 ==="
curl -s "http://admin:admin@127.0.0.1:7002/_api/database/testdb/collection/users/stats" | jq '.cluster.shards'

echo ""
echo "=== Stats from node 3 ==="
curl -s "http://admin:admin@127.0.0.1:7003/_api/database/testdb/collection/users/stats" | jq '.cluster.shards'

# Check logs for import messages
echo ""
echo "=== Import log messages from node 3 ==="
grep -E "\[IMPORT\]" /tmp/import_test_data/n3.log | tail -20

# Cleanup
echo ""
echo "Cleaning up..."
kill $PID1 $PID2 $PID3 2>/dev/null || true

echo "Done!"
