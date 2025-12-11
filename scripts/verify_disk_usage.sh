#!/bin/bash
set -e

# Set consistent secret for authentication across all nodes
export JWT_SECRET="test-secret-for-sharding-verification-12345"
export SOLIDB_ADMIN_PASSWORD="admin"

# Cleanup any previous run
pkill -f "solidb" || true
sleep 1
rm -rf tmp/n1 tmp/n2 tmp/n3 || { echo "Cleanup failed"; exit 1; }
mkdir -p tmp/n1 tmp/n2 tmp/n3

# Compile first
echo "Compiling..."
cargo build

BIN=./target/debug/solidb

# Start Node 1 (Bootstrap)
echo "Starting Node 1..."
$BIN --port 8001 --replication-port 9001 --data-dir ./tmp/n1 > tmp/n1.log 2>&1 &
PID1=$!
sleep 2

# Start Node 2
echo "Starting Node 2..."
$BIN --port 8002 --replication-port 9002 --peer 127.0.0.1:9001 --data-dir ./tmp/n2 > tmp/n2.log 2>&1 &
PID2=$!
sleep 2

# Start Node 3
echo "Starting Node 3..."
$BIN --port 8003 --replication-port 9003 --peer 127.0.0.1:9001 --data-dir ./tmp/n3 > tmp/n3.log 2>&1 &
PID3=$!
sleep 5 # Wait for cluster sync

echo "Cluster started. PIDs: $PID1, $PID2, $PID3"

# Create Unsharded Collection (users) on Node 1
echo "Creating unsharded collection 'users' on Node 1..."
curl -s -u admin:admin -X POST http://localhost:8001/_api/database/_system/collection \
  -H "Content-Type: application/json" \
  -d '{"name": "users", "type": "document"}'

# Create Sharded Collection (users2) on Node 1
echo "Creating sharded collection 'users2' on Node 1..."
curl -s -u admin:admin -X POST http://localhost:8001/_api/database/_system/collection \
  -H "Content-Type: application/json" \
  -d '{"name": "users2", "type": "document", "numShards": 3, "replicationFactor": 2, "shardKey": "_key"}'

sleep 2

# Helper to insert data
insert_docs() {
    COLL=$1
    echo "Inserting 300 documents into $COLL (in parallel)..."
    for i in {1..300}; do
        # Use a reasonably large payload to make disk usage visible
        PAYLOAD=$(printf 'x%.0s' {1..1000}) # 1KB string
        curl -s -u admin:admin -X POST "http://localhost:8001/_api/database/_system/document/$COLL" \
          -H "Content-Type: application/json" \
          -d "{\"_key\": \"doc$i\", \"value\": \"$PAYLOAD\"}" > /dev/null &
        
        # Limit parallelism to avoid overwhelming the server
        if (( i % 20 == 0 )); then wait; fi
    done
    wait
}

insert_docs "users"
insert_docs "users2"

sleep 5 # Wait for replication/sharding sync

echo "Disk Usage:"
du -sh tmp/n1
du -sh tmp/n2
du -sh tmp/n3

echo "Detailed usage in tmp/n1:"
ls -R tmp/n1 | grep users || true

# Cleanup
kill $PID1 $PID2 $PID3 || true
