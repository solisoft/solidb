#!/bin/bash
# Simpler test - create collection and verify shard config

set -e
cd "$(dirname "$0")/.."

BIN="./target/debug/solidb"
export SOLIDB_ADMIN_PASSWORD="admin"

# Clean up
rm -rf /tmp/simple_test_data
mkdir -p /tmp/simple_test_data

echo "Starting single node..."
$BIN --port 7010 --data-dir /tmp/simple_test_data/n1 > /tmp/simple_test_data/n1.log 2>&1 &
PID1=$!
sleep 3

echo "Creating database..."
curl -s -X POST "http://admin:admin@127.0.0.1:7010/_api/database" \
    -H "Content-Type: application/json" \
    -d '{"name": "testdb"}'
echo ""

echo "Creating sharded collection..."
curl -s -X POST "http://admin:admin@127.0.0.1:7010/_api/database/testdb/collection" \
    -H "Content-Type: application/json" \
    -d '{"name": "users", "numShards": 3, "replicationFactor": 2}'
echo ""

sleep 1

echo "Checking stats..."
curl -s "http://admin:admin@127.0.0.1:7010/_api/database/testdb/collection/users/stats" | jq '.sharding'
echo ""

echo "Log entries about shard config:"
grep -E "(shard|SHARD)" /tmp/simple_test_data/n1.log || echo "No shard entries found"

kill $PID1 2>/dev/null || true
echo "Done!"
