#!/bin/bash
# Reproduce the 3-to-4 shard resharding issue

set -e
cd "$(dirname "$0")/.."

BIN="./target/debug/solidb"
export SOLIDB_ADMIN_PASSWORD="admin"
export JWT_SECRET="test-secret-for-resharding-12345"

# Cleanup
pkill -f "solidb" || true
sleep 1
rm -rf tmp/node{1,2,3,4} || true
mkdir -p tmp/node{1,2,3,4}

echo "Compiling..."
cargo build --quiet 2>/dev/null

echo "Starting 4-node cluster..."
$BIN --port 8001 --replication-port 9001 --data-dir ./tmp/node1 > tmp/node1.log 2>&1 &
PID1=$!
sleep 2
$BIN --port 8002 --replication-port 9002 --peer 127.0.0.1:9001 --data-dir ./tmp/node2 > tmp/node2.log 2>&1 &
PID2=$!
sleep 2
$BIN --port 8003 --replication-port 9003 --peer 127.0.0.1:9001 --data-dir ./tmp/node3 > tmp/node3.log 2>&1 &
PID3=$!
sleep 2
$BIN --port 8004 --replication-port 9004 --peer 127.0.0.1:9001 --data-dir ./tmp/node4 > tmp/node4.log 2>&1 &
PID4=$!
sleep 5

echo "Cluster PIDs: $PID1 $PID2 $PID3 $PID4"

# Create database and sharded collection with 3 shards
echo "Creating database and 3-shard collection..."
curl -s -u admin:admin -X POST http://localhost:8001/_api/database \
  -H "Content-Type: application/json" -d '{"name": "testdb"}' > /dev/null

curl -s -u admin:admin -X POST http://localhost:8001/_api/database/testdb/collection \
  -H "Content-Type: application/json" -d '{"name": "users", "numShards": 3, "replicationFactor": 2}' > /dev/null

sleep 3

# Insert test documents
echo "Inserting 1000 test documents..."
for i in $(seq 1 1000); do
  curl -s -u admin:admin -X POST \
    "http://localhost:8001/_api/database/testdb/document/users" \
    -H "Content-Type: application/json" \
    -d "{\"_key\": \"user_$i\", \"name\": \"User $i\", \"data\": \"Some test data for user $i\"}" > /dev/null &
  if (( i % 50 == 0 )); then wait; fi
done
wait

sleep 5

# Check document count before resharding
echo "Document count before resharding:"
COUNT_BEFORE=$(curl -s -u admin:admin "http://localhost:8001/_api/database/testdb/collection/users/stats" | jq -r '.document_count // 0')
echo "Documents: $COUNT_BEFORE"

# Trigger resharding from 3 to 4 shards
echo "Triggering resharding from 3 to 4 shards..."
curl -s -u admin:admin -X PUT "http://localhost:8001/_api/database/testdb/collection/users" \
  -H "Content-Type: application/json" \
  -d '{"numShards": 4, "replicationFactor": 2}' > /dev/null

echo "Waiting for resharding to complete (this may take time)..."
sleep 30

# Check document count after resharding
echo "Document count after resharding:"
COUNT_AFTER=$(curl -s -u admin:admin "http://localhost:8001/_api/database/testdb/collection/users/stats" | jq -r '.document_count // 0')
echo "Documents: $COUNT_AFTER"

if [ "$COUNT_BEFORE" != "$COUNT_AFTER" ]; then
  echo "ERROR: Document count changed from $COUNT_BEFORE to $COUNT_AFTER!"
  echo "This indicates data loss during resharding."
else
  echo "SUCCESS: Document count remained stable at $COUNT_AFTER"
fi

# Check for any errors in logs
echo ""
echo "Checking logs for errors..."
for node in 1 2 3 4; do
  echo "Node $node errors:"
  grep -i "error\|fail" tmp/node${node}.log | tail -5 || echo "No errors found"
  echo ""
done

# Check sharding status
echo "Final sharding status:"
curl -s -u admin:admin "http://localhost:8001/_api/database/testdb/collection/users/sharding" | jq .

# Cleanup
kill $PID1 $PID2 $PID3 $PID4 2>/dev/null || true

echo "Test complete."
