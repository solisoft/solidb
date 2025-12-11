#!/bin/bash
set -e
export SOLIDB_ADMIN_PASSWORD="admin"
pkill -f "solidb" || true
rm -rf tmp/n1 tmp/n2 tmp/n3
mkdir -p tmp/n1 tmp/n2 tmp/n3
echo "Compiling..."
cargo build
BIN=./target/debug/solidb

echo "Starting N1..."
$BIN --port 8001 --replication-port 9001 --data-dir ./tmp/n1 > tmp/n1.log 2>&1 &
PID1=$!
sleep 2

echo "Starting N2..."
$BIN --port 8002 --replication-port 9002 --peer 127.0.0.1:9001 --data-dir ./tmp/n2 > tmp/n2.log 2>&1 &
PID2=$!
sleep 2

echo "Starting N3..."
$BIN --port 8003 --replication-port 9003 --peer 127.0.0.1:9001 --data-dir ./tmp/n3 > tmp/n3.log 2>&1 &
PID3=$!
sleep 15
echo "Cluster started. PIDs: $PID1, $PID2, $PID3"

echo "Creating sharded collection..."
curl -s -u admin:admin -X POST http://localhost:8001/_api/database/_system/collection \
  -H "Content-Type: application/json" \
  -d '{"name": "test_sharded", "type": "document", "numShards": 3, "replicationFactor": 2, "shardKey": "_key"}'

sleep 2
echo "Inserting 99 documents..."
# Use simple loop
for i in {1..99}; do
  curl -s -u admin:admin -X POST http://localhost:8001/_api/database/_system/document/test_sharded \
    -H "Content-Type: application/json" \
    -d "{\"_key\": \"doc$i\", \"value\": \"v$i\"}" > /dev/null
done

sleep 5 # Wait for replication

echo "Killing Node 2 ($PID2) and Node 3 ($PID3)..."
kill -9 $PID2 $PID3
sleep 5

echo "Querying Node 1..."
RESPONSE=$(curl -s -u admin:admin -X POST http://localhost:8001/_api/database/_system/cursor \
  -H "Content-Type: application/json" \
  -d '{"query": "FOR doc IN test_sharded RETURN doc"}')

COUNT=$(echo $RESPONSE | grep -o "\"_key\"" | wc -l)
# Clean up whitespace
COUNT=$(echo $COUNT | xargs)
echo "Node 1 reports $COUNT documents."

# Kill N1
kill $PID1 || true

# Check if count is >= 60 (allow some variance for distribution/hashing). Expect ~66.
# If replication FAILED, count will be ~33.

if [ "$COUNT" -ge 60 ]; then
  echo "SUCCESS: Found $COUNT documents (expected approx 66)."
  exit 0
else
  echo "FAILURE: Found $COUNT documents (expected approx 66). Replication likely failed."
  exit 1
fi
