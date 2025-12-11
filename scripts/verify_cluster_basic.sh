#!/bin/bash
set -e

# Cleanup any previous instances
pkill -f "solidb" || true
rm -rf ./verify_data

# Ensure we have a fresh build
cargo build

# Start server in background with explicit password
echo "Starting SolidDB..."
SOLIDB_ADMIN_PASSWORD=admin nohup ./target/debug/solidb --data-dir ./verify_data --port 6745 > solidb.log 2>&1 &
SERVER_PID=$!

function cleanup {
    echo "Stopping server..."
    kill $SERVER_PID || true
    cat solidb.log
}
trap cleanup EXIT

# Wait for server
echo "Waiting for server to start..."
for i in {1..10}; do
    if curl -s http://localhost:6745/_api/status > /dev/null; then
        echo "Server is up!"
        break
    fi
    sleep 1
done

# Basic Auth
AUTH="Authorization: Basic $(echo -n 'admin:admin' | base64)"

# Check Cluster Status
echo "Checking Cluster Status..."
curl -s -H "$AUTH" http://localhost:6745/_api/cluster/status | jq .

# Create Collection
echo "Creating Collection 'test_coll' with sharding..."
curl -X POST -H "$AUTH" -H "Content-Type: application/json" \
     -d '{"name": "test_coll", "num_shards": 2, "replication_factor": 1}' \
     http://localhost:6745/_api/database/_system/collections | jq .

# Insert Document
echo "Inserting Document..."
curl -X POST -H "$AUTH" -H "Content-Type: application/json" \
     -d '{"foo": "bar"}' \
     http://localhost:6745/_api/database/_system/collection/test_coll/documents | jq .

# Verify Document Read (This implicitly uses ShardRouter/Coordinator)
echo "Reading Document..."
# First scan to get key
KEY=$(curl -s -H "$AUTH" http://localhost:6745/_api/database/_system/collection/test_coll/documents | jq -r '.[0]._key')

echo "Found key: $KEY"
curl -s -H "$AUTH" http://localhost:6745/_api/database/_system/collection/test_coll/document/$KEY | jq .

echo "Verification Complete!"
