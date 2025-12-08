#!/bin/bash
set -e

# Setup
DB_NAME="test_dump_db"
RESTORE_DB_NAME="test_restore_db"
COLL_NAME="blob_coll"
API_URL="http://localhost:6755"
AUTH="admin:admin"

# Colors
GREEN='\033[0;32m'
RED='\033[0;31m'
NC='\033[0m'

echo "Starting verification of Blob Dump/Restore..."

# 1. Create Database
echo "Creating database $DB_NAME..."
curl -s -u "$AUTH" -X POST "$API_URL/_api/database" -H "Content-Type: application/json" -d "{\"name\": \"$DB_NAME\"}" > /dev/null

# 2. Create Blob Collection
echo "Creating blob collection $COLL_NAME..."
curl -s -u "$AUTH" -X POST "$API_URL/_api/database/$DB_NAME/collection" \
  -H "Content-Type: application/json" \
  -d "{\"name\": \"$COLL_NAME\", \"type\": \"blob\"}" > /dev/null

# 3. Upload Blob (Simulating upload of a small binary file)
echo "Creating test blob file..."
dd if=/dev/urandom of=test_blob.bin bs=1024 count=100 2>/dev/null # 100KB blob
MD5_ORIG=$(md5 -q test_blob.bin)

echo "Uploading blob..."
curl -s -u "$AUTH" -X POST "$API_URL/_api/blob/$DB_NAME/$COLL_NAME" \
  -F "file=@test_blob.bin" > response.json

DOC_KEY=$(cat response.json | grep -o '"_key":"[^"]*"' | cut -d'"' -f4)
echo "Blob uploaded with key: $DOC_KEY"

if [ -z "$DOC_KEY" ]; then
    echo -e "${RED}Upload failed${NC}"
    cat response.json
    exit 1
fi

# 4. Dump Collection
echo "Dumping collection..."
./target/debug/solidb-dump --host localhost --port 6755 --username admin --password admin \
  --database "$DB_NAME" --collection "$COLL_NAME" --output dump.jsonl

if [ ! -s dump.jsonl ]; then
    echo -e "${RED}Dump file empty or missing${NC}"
    exit 1
fi

echo "Dump file size: $(ls -lh dump.jsonl | awk '{print $5}')"

# 5. Restore to New Database
echo "Restoring to $RESTORE_DB_NAME..."
./target/debug/solidb-restore --host localhost --port 6755 --username admin --password admin \
  --input dump.jsonl --database "$RESTORE_DB_NAME" --create-database --drop

# 6. Verify Content
echo "Verifying restored blob..."
curl -s -u "$AUTH" "$API_URL/_api/blob/$RESTORE_DB_NAME/$COLL_NAME/$DOC_KEY" -o restored_blob.bin

MD5_RESTORE=$(md5 -q restored_blob.bin)

if [ "$MD5_ORIG" == "$MD5_RESTORE" ]; then
    echo -e "${GREEN}Verification SUCCESS: Checksums match ($MD5_ORIG)${NC}"
else
    echo -e "${RED}Verification FAILED: Checksums differ!${NC}"
    echo "Original: $MD5_ORIG"
    echo "Restored: $MD5_RESTORE"
    exit 1
fi

# Cleanup
echo "Cleaning up..."
rm test_blob.bin restored_blob.bin dump.jsonl response.json
curl -s -u "$AUTH" -X DELETE "$API_URL/_api/database/$DB_NAME" > /dev/null
curl -s -u "$AUTH" -X DELETE "$API_URL/_api/database/$RESTORE_DB_NAME" > /dev/null

echo "Done."
