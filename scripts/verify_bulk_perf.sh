#!/bin/bash
set -e

# Base URL
API_URL="http://localhost:6745"
DB="_system"
COLL="perf_test_users"

# clean up
echo "Cleaning up..."
curl -s -X DELETE "$API_URL/_api/database/$DB/collection/$COLL" > /dev/null || true

# create collection
echo "Creating collection..."
curl -s -X POST "$API_URL/_api/database/$DB/collection" \
  -H "Content-Type: application/json" \
  -d "{\"name\": \"$COLL\"}" > /dev/null

# Insert 100,000 documents
echo "Inserting 100,000 documents..."
START_TIME=$(date +%s%N)

curl -s -X POST "$API_URL/_api/database/$DB/cursor" \
  -H "Content-Type: application/json" \
  -d "{\"query\": \"FOR i IN 1..100000 INSERT { email: CONCAT('user', i, '@example.com') } INTO $COLL\"}" > /dev/null

END_TIME=$(date +%s%N)
DURATION=$(( (END_TIME - START_TIME) / 1000000 ))

echo "Insert took ${DURATION}ms"

# Verify count
COUNT=$(curl -s "$API_URL/_api/database/$DB/collection/$COLL/stats" | grep -o '"count":[0-9]*' | grep -o '[0-9]*')
echo "Collection count: $COUNT"

if [ "$COUNT" -eq "100000" ]; then
    echo "SUCCESS: Inserted 100,000 documents"
else
    echo "FAILURE: Expected 100,000 documents, got $COUNT"
    exit 1
fi
