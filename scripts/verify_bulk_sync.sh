#!/bin/bash
# Verification script for bulk insert replication sync
# Tests that 10k documents inserted on node1 sync correctly to node2 and node3

# Don't exit on error - we handle errors explicitly

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Configuration
export JWT_SECRET="test-secret-key-12345678901234"
export SOLIDB_ADMIN_PASSWORD="admin"
DOC_COUNT=100000
TIMEOUT=120
DB_NAME="testdb"
COLL_NAME="sync_test"

echo -e "${YELLOW}=== Bulk Sync Verification Test ===${NC}"
echo "Inserting $DOC_COUNT documents and verifying replication"

# Cleanup function
cleanup() {
    echo -e "\n${YELLOW}Cleaning up...${NC}"
    kill $PID1 $PID2 $PID3 2>/dev/null || true
    # rm -rf /tmp/solidb_sync_test_*
}
trap cleanup EXIT

# Create data directories
rm -rf /tmp/solidb_sync_test_*
mkdir -p /tmp/solidb_sync_test_1
mkdir -p /tmp/solidb_sync_test_2
mkdir -p /tmp/solidb_sync_test_3

# Build if needed
echo -e "${YELLOW}Building...${NC}"
cargo build --release 2>/dev/null || cargo build

# Start Node 1 (bootstrap)
echo -e "${YELLOW}Starting Node 1 (port 6745)...${NC}"
RUST_LOG=solidb=debug,tower_http=info ./target/release/solidb -p 6745 --replication-port 7745 --data-dir /tmp/solidb_sync_test_1 > /tmp/solidb_sync_test_1/log.txt 2>&1 &
PID1=$!
sleep 2

# Start Node 2
echo -e "${YELLOW}Starting Node 2 (port 6746)...${NC}"
./target/release/solidb -p 6746 --replication-port 7746 --peer localhost:7745 --data-dir /tmp/solidb_sync_test_2 > /tmp/solidb_sync_test_2/log.txt 2>&1 &
PID2=$!
sleep 2

# Start Node 3
echo -e "${YELLOW}Starting Node 3 (port 6747)...${NC}"
./target/release/solidb -p 6747 --replication-port 7747 --peer localhost:7745 --data-dir /tmp/solidb_sync_test_3 > /tmp/solidb_sync_test_3/log.txt 2>&1 &
PID3=$!
sleep 3

# Create test database on Node 1
echo -e "${YELLOW}Creating database '$DB_NAME' on Node 1...${NC}"
CREATE_DB=$(curl -s -u admin:admin -X POST "http://localhost:6745/_api/database" \
    -H "Content-Type: application/json" \
    -d "{\"name\": \"$DB_NAME\"}")
echo "Create DB response: $CREATE_DB"

# Create collection using SolidB API: /_api/database/{db}/collection
echo -e "${YELLOW}Creating collection '$COLL_NAME' on Node 1...${NC}"
CREATE_COLL=$(curl -s -u admin:admin -X POST "http://localhost:6745/_api/database/$DB_NAME/collection" \
    -H "Content-Type: application/json" \
    -d "{\"name\": \"$COLL_NAME\"}")
echo "Create Collection response: $CREATE_COLL"

sleep 1

# Insert documents on Node 1 using bulk SDBQL via cursor endpoint
echo -e "${YELLOW}Inserting $DOC_COUNT documents on Node 1...${NC}"
START_TIME=$(date +%s)

# Generate and insert documents using /_api/database/{db}/cursor
# Use a simpler query without nested quotes
RESULT=$(curl -s -u admin:admin -X POST "http://localhost:6745/_api/database/$DB_NAME/cursor" \
    -H "Content-Type: application/json" \
    -d '{"query": "FOR i IN 1..'"$DOC_COUNT"' INSERT { value: i, status: \"active\" } INTO '"$COLL_NAME"'"}')

INSERT_TIME=$(date +%s)
INSERT_DURATION=$((INSERT_TIME - START_TIME))
echo "Insert result: $RESULT"
echo -e "${GREEN}Insert completed in ${INSERT_DURATION}s${NC}"

# Verify document count on Node 1 using collection stats
RESPONSE1=$(curl -s -u admin:admin "http://localhost:6745/_api/database/$DB_NAME/collection/$COLL_NAME/stats")
echo "Node 1 stats response: $RESPONSE1"
COUNT1=$(echo "$RESPONSE1" | grep -o '"document_count":[0-9]*' | head -1 | grep -o '[0-9]*' || echo "0")
COUNT1=${COUNT1:-0}
echo "Node 1 count: $COUNT1"

if [ "$COUNT1" = "0" ]; then
    echo -e "${RED}ERROR: Insert failed - Node 1 has 0 documents${NC}"
    echo -e "Check logs at /tmp/solidb_sync_test_1/log.txt"
    tail -50 /tmp/solidb_sync_test_1/log.txt
    exit 1
fi

# Wait for sync with timeout
echo -e "${YELLOW}Waiting for sync to complete (timeout: ${TIMEOUT}s)...${NC}"
SYNC_START=$(date +%s)

while true; do
    ELAPSED=$(($(date +%s) - SYNC_START))
    
    # Get counts from Node 2 and Node 3 using stats endpoint
    COUNT2=$(curl -s -u admin:admin "http://localhost:6746/_api/database/$DB_NAME/collection/$COLL_NAME/stats" 2>/dev/null | grep -o '"document_count":[0-9]*' | head -1 | grep -o '[0-9]*' || echo "0")
    COUNT3=$(curl -s -u admin:admin "http://localhost:6747/_api/database/$DB_NAME/collection/$COLL_NAME/stats" 2>/dev/null | grep -o '"document_count":[0-9]*' | head -1 | grep -o '[0-9]*' || echo "0")
    
    # Handle empty responses
    COUNT2=${COUNT2:-0}
    COUNT3=${COUNT3:-0}
    
    echo -ne "\rNode1: $COUNT1 | Node2: $COUNT2 | Node3: $COUNT3 | Elapsed: ${ELAPSED}s   "
    
    # Check if sync is complete
    if [ "$COUNT2" = "$COUNT1" ] && [ "$COUNT3" = "$COUNT1" ]; then
        SYNC_DURATION=$ELAPSED
        echo -e "\n${GREEN}✓ Sync completed in ${SYNC_DURATION}s${NC}"
        break
    fi
    
    # Timeout check
    if [ $ELAPSED -ge $TIMEOUT ]; then
        echo -e "\n${RED}✗ Sync timeout after ${TIMEOUT}s${NC}"
        echo -e "Node 1: $COUNT1"
        echo -e "Node 2: $COUNT2"
        echo -e "Node 3: $COUNT3"
        echo -e "\nNode 1 logs (last 30 lines):"
        tail -30 /tmp/solidb_sync_test_1/log.txt
        echo -e "\nNode 2 logs (last 30 lines):"
        tail -30 /tmp/solidb_sync_test_2/log.txt
        exit 1
    fi
    
    sleep 1
done

# Final verification
echo -e "\n${YELLOW}Final verification...${NC}"

FINAL_COUNT1=$(curl -s -u admin:admin "http://localhost:6745/_api/database/$DB_NAME/collection/$COLL_NAME/stats" | grep -o '"document_count":[0-9]*' | head -1 | grep -o '[0-9]*' || echo "0")
FINAL_COUNT2=$(curl -s -u admin:admin "http://localhost:6746/_api/database/$DB_NAME/collection/$COLL_NAME/stats" | grep -o '"document_count":[0-9]*' | head -1 | grep -o '[0-9]*' || echo "0")
FINAL_COUNT3=$(curl -s -u admin:admin "http://localhost:6747/_api/database/$DB_NAME/collection/$COLL_NAME/stats" | grep -o '"document_count":[0-9]*' | head -1 | grep -o '[0-9]*' || echo "0")

echo "Node 1: $FINAL_COUNT1 documents"
echo "Node 2: $FINAL_COUNT2 documents"
echo "Node 3: $FINAL_COUNT3 documents"

# Verify all counts match
if [ "$FINAL_COUNT1" = "$DOC_COUNT" ] && [ "$FINAL_COUNT2" = "$DOC_COUNT" ] && [ "$FINAL_COUNT3" = "$DOC_COUNT" ]; then
    echo -e "\n${GREEN}✓ SUCCESS: All nodes have $DOC_COUNT documents${NC}"
    echo -e "${GREEN}  Insert time: ${INSERT_DURATION}s${NC}"
    echo -e "${GREEN}  Sync time: ${SYNC_DURATION}s${NC}"
    TOTAL=$((INSERT_DURATION + SYNC_DURATION))
    if [ $TOTAL -gt 0 ]; then
        echo -e "${GREEN}  Total time: ${TOTAL}s${NC}"
        echo -e "${GREEN}  Throughput: $((DOC_COUNT / TOTAL)) docs/sec (end-to-end)${NC}"
    fi
    exit 0
else
    echo -e "\n${RED}✗ FAILED: Document counts don't match!${NC}"
    exit 1
fi
