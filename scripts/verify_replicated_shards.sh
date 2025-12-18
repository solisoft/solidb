#!/bin/bash

# Configuration
NODE1="http://localhost:6745"
NODE2="http://localhost:6746"
NODE3="http://localhost:6747"
COLL="sharded_sync_verif"

echo "=== Verifying Sharded Sync ==="

# 0. Authenticate
# If first argument is provided, use it as token
if [ -n "$1" ]; then
    TOKEN="$1"
    echo "Using provided token: ${TOKEN:0:10}..."
    AUTH_HEADER="Authorization: Bearer $TOKEN"
else
    echo "Authenticating as admin/${SOLIDB_ADMIN_PASSWORD:-password}..."
    TOKEN=$(curl -s -X POST "$NODE1/auth/login" \
      -H "Content-Type: application/json" \
      -d "{\"username\": \"admin\", \"password\": \"${SOLIDB_ADMIN_PASSWORD:-password}\"}" | jq -r .token)
    
    if [ "$TOKEN" == "null" ] || [ -z "$TOKEN" ]; then
        echo "Auth with default credentials failed."
        echo "Usage: ./scripts/verify_replicated_shards.sh [YOUR_ADMIN_TOKEN]"
        echo "Continuing without token (might fail with 401)..."
        AUTH_HEADER=""
    else
        echo "Got token: ${TOKEN:0:10}..."
        AUTH_HEADER="Authorization: Bearer $TOKEN"
    fi
fi

# 1. Create Database
echo "Creating database testdb..."
curl -s -X POST "$NODE1/_api/database" \
  -H "Content-Type: application/json" \
  -H "$AUTH_HEADER" \
  -d '{"name": "testdb"}'

# 2. Create Sharded Collection (3 shards, 2 replicas)
echo "Creating collection $COLL..."
curl -s -X POST "$NODE1/_api/database/testdb/collection" \
  -H "Content-Type: application/json" \
  -H "$AUTH_HEADER" \
  -d '{"name": "'"$COLL"'", "numShards": 3, "replicationFactor": 2}' | jq .

sleep 2

# 3. Insert 30 Documents
echo "Inserting 30 documents..."
for i in {1..30}; do
  curl -s -X POST "$NODE1/_api/database/testdb/document/$COLL" \
    -H "Content-Type: application/json" \
    -H "$AUTH_HEADER" \
    -d "{\"_key\": \"doc_$i\", \"value\": $i}" > /dev/null
done
echo "Insertion complete."

sleep 2

# 4. Check Distribution (Logical)
echo "Checking logical count on Node 1:"
COUNT=$(curl -s -H "$AUTH_HEADER" "$NODE1/_api/database/testdb/collection/$COLL/stats" | jq .document_count)
echo "Logical Count: $COUNT"

# 5. Check Physical Shards
# Note: Physical shards are collections like 'sharded_sync_verif_s0', 's1', etc.
# We list collections in 'testdb' and filter.

echo "Checking Physical Shards..."
echo "Node 1 Shards:"
curl -s -H "$AUTH_HEADER" "$NODE1/_api/database/testdb/collection" | jq -r ".collections[] | select(.name | startswith(\"${COLL}_s\")) | .name + \": \" + (.count|tostring)"

echo "Node 2 Shards:"
curl -s -H "$AUTH_HEADER" "$NODE2/_api/database/testdb/collection" | jq -r ".collections[] | select(.name | startswith(\"${COLL}_s\")) | .name + \": \" + (.count|tostring)"

echo "Node 3 Shards:"
curl -s -H "$AUTH_HEADER" "$NODE3/_api/database/testdb/collection" | jq -r ".collections[] | select(.name | startswith(\"${COLL}_s\")) | .name + \": \" + (.count|tostring)"

# 5. Check Total Physical Count
echo "--- Summary ---"
# Note: Shards might overlap if replicas are present on same node? No, usually 1 replica per node per shard.
# If replicationFactor=2, we have Primary + 1 Replica = 2 copies total.
# Total physical docs should be 30 * 2 = 60.

echo "Done."
