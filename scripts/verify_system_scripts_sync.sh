#!/bin/bash
set -e

# Set consistent secret for authentication across all nodes
export JWT_SECRET="test-secret-for-script-sync-verification-12345"
export SOLIDB_ADMIN_PASSWORD="admin"

# Cleanup any previous run
pkill -f "solidb" || true
rm -rf tmp/n1 tmp/n2
mkdir -p tmp/n1 tmp/n2

# Compile first
echo "Compiling..."
cargo build

BIN=./target/debug/solidb

# Start Node 1 (Bootstrap)
echo "Starting Node 1..."
$BIN --port 8011 --replication-port 9011 --data-dir ./tmp/n1 > tmp/n1.log 2>&1 &
PID1=$!
sleep 2

# Start Node 2
echo "Starting Node 2..."
$BIN --port 8012 --replication-port 9012 --peer 127.0.0.1:9011 --data-dir ./tmp/n2 > tmp/n2.log 2>&1 &
PID2=$!
sleep 5 # Wait for cluster sync

echo "Cluster started. PIDs: $PID1, $PID2"

# Define a test script
SCRIPT_NAME="test_sync_script"
SCRIPT_CODE='return { message = "hello from cluster" }'

# Create a script on Node 1
echo "Creating script on Node 1..."
RESPONSE=$(curl -s -X POST -u admin:admin http://localhost:8011/_api/database/_system/scripts \
  -H "Content-Type: application/json" \
  -d "{\"name\": \"$SCRIPT_NAME\", \"path\": \"mylib\", \"methods\": [\"GET\"], \"code\": \"return 'Hello from replicated script'\"}")
echo "Create Script Response: $RESPONSE"

# Also create a normal collection and document to verify basic replication works
echo "Creating normal collection and document on Node 1..."
curl -s -u admin:admin -X POST http://localhost:8011/_api/database/_system/collections \
  -H "Content-Type: application/json" \
  -d '{"name": "test_repl_check"}'

curl -s -u admin:admin -X POST http://localhost:8011/_api/database/_system/collection/test_repl_check/documents \
  -H "Content-Type: application/json" \
  -d '{"_key": "doc1", "val": "replication_works"}'

sleep 15

# Verify Script on Node 2
echo "Checking script availability on Node 2..."
# List scripts (Management API)
LIST_RESPONSE=$(curl -s -u admin:admin http://localhost:8012/_api/database/_system/scripts)
echo "List Scripts Response: $LIST_RESPONSE"

# Execute Script (Custom API)
EXEC_RESPONSE=$(curl -s -u admin:admin http://localhost:8012/_api/custom/_system/mylib)
echo "Execute Script Response: $EXEC_RESPONSE"

# Verify Document on Node 2
echo "Checking document availability on Node 2..."
DOC_RESPONSE=$(curl -s -u admin:admin http://localhost:8012/_api/database/_system/collection/test_repl_check/document/doc1)
echo "Response from Node 2 (Doc): $DOC_RESPONSE"

# Cleanup
kill $PID1 $PID2 || true

# Verification
if [[ "$DOC_RESPONSE" == *"replication_works"* ]]; then
  echo "Basic replication works."
else
  echo "FAILURE: Basic replication FAILED. Cluster issue?"
  echo "--- Node 1 Log ---"
  cat tmp/n1.log | tail -n 20
  echo "--- Node 2 Log ---"
  cat tmp/n2.log | tail -n 20
fi

if [[ "$LIST_RESPONSE" == *"$SCRIPT_NAME"* ]]; then
  echo "Script found in list on Node 2."
else
  echo "FAILURE: Script NOT found in list on Node 2."
  exit 1
fi

if [[ "$EXEC_RESPONSE" == *"Hello from replicated script"* ]]; then
  echo "Script execution successful on Node 2."
else 
  echo "FAILURE: Script execution FAILED on Node 2."
fi
  echo "SUCCESS: Script created on Node 1 is visible on Node 2!"
  exit 0
else
  echo "FAILURE: Script NOT found on Node 2."
  exit 1
fi
