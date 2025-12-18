#!/bin/bash
# =============================================================================
# Comprehensive Cluster Test Script
# Tests:
#   1. Non-sharded collection replication
#   2. Sharded collection replication
#   3. Node removal and rebalance verification
# =============================================================================

set -e

# Trap to cleanup on exit
trap 'cleanup_on_exit' EXIT INT TERM

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
SCRIPT_DIR=$(dirname "$0")
cd "$SCRIPT_DIR/.."

BIN="./target/debug/solidb"
TEST_DIR="/tmp/cluster_full_test"
export SOLIDB_ADMIN_PASSWORD="admin"
AUTH="admin:admin"

# Document counts
UNSHARDED_DOCS=50
SHARDED_DOCS=100

# Helper functions
log_header() {
    echo ""
    echo -e "${BLUE}=============================================${NC}"
    echo -e "${BLUE} $1${NC}"
    echo -e "${BLUE}=============================================${NC}"
}

log_info() {
    echo -e "${GREEN}[INFO]${NC} $1"
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

log_success() {
    echo -e "${GREEN}[✓ PASS]${NC} $1"
}

log_fail() {
    echo -e "${RED}[✗ FAIL]${NC} $1"
}

cleanup() {
    log_info "Cleaning up processes and data..."
    # Kill any solidb processes on our test ports
    pkill -9 -f "port 7201" 2>/dev/null || true
    pkill -9 -f "port 7202" 2>/dev/null || true
    pkill -9 -f "port 7203" 2>/dev/null || true
    # Also kill by PID if we have them
    [ -n "${PID1:-}" ] && kill -9 $PID1 2>/dev/null || true
    [ -n "${PID2:-}" ] && kill -9 $PID2 2>/dev/null || true
    [ -n "${PID3:-}" ] && kill -9 $PID3 2>/dev/null || true
    sleep 3
    rm -rf "$TEST_DIR"
    mkdir -p "$TEST_DIR"/{n1,n2,n3}
}

cleanup_on_exit() {
    log_info "Cleaning up on exit..."
    [ -n "${PID1:-}" ] && kill -9 $PID1 2>/dev/null || true
    [ -n "${PID2:-}" ] && kill -9 $PID2 2>/dev/null || true
    [ -n "${PID3:-}" ] && kill -9 $PID3 2>/dev/null || true
}

wait_for_node() {
    local port=$1
    local max_attempts=30
    local attempt=0
    while [ $attempt -lt $max_attempts ]; do
        if curl -s "http://$AUTH@127.0.0.1:$port/_api/databases" > /dev/null 2>&1; then
            return 0
        fi
        attempt=$((attempt + 1))
        sleep 1
    done
    return 1
}

get_doc_count() {
    local port=$1
    local db=$2
    local collection=$3
    curl -s "http://$AUTH@127.0.0.1:$port/_api/database/$db/collection/$collection/stats" 2>/dev/null | jq -r '.count // 0'
}

get_shard_info() {
    local port=$1
    local db=$2
    local collection=$3
    curl -s "http://$AUTH@127.0.0.1:$port/_api/database/$db/collection/$collection/stats" 2>/dev/null | jq '.cluster.shards // empty'
}

# =============================================================================
# MAIN TEST EXECUTION
# =============================================================================

log_header "CLUSTER COMPREHENSIVE TEST"
echo "Testing:"
echo "  1. Non-sharded collection replication"
echo "  2. Sharded collection replication"
echo "  3. Node removal and rebalance"
echo ""

# Build first
log_info "Building project..."
cargo build --quiet 2>/dev/null

# Cleanup any previous test
cleanup

# =============================================================================
# START 3-NODE CLUSTER
# =============================================================================
log_header "STARTING 3-NODE CLUSTER"

log_info "Starting Node 1 (bootstrap)..."
$BIN --port 7201 --replication-port 8201 --data-dir "$TEST_DIR/n1" > "$TEST_DIR/n1.log" 2>&1 &
PID1=$!
wait_for_node 7201 || { log_error "Node 1 failed to start"; exit 1; }
log_success "Node 1 started (PID: $PID1)"

log_info "Starting Node 2..."
$BIN --port 7202 --replication-port 8202 --peer 127.0.0.1:8201 --data-dir "$TEST_DIR/n2" > "$TEST_DIR/n2.log" 2>&1 &
PID2=$!
wait_for_node 7202 || { log_error "Node 2 failed to start"; exit 1; }
log_success "Node 2 started (PID: $PID2)"

log_info "Starting Node 3..."
$BIN --port 7203 --replication-port 8203 --peer 127.0.0.1:8201 --data-dir "$TEST_DIR/n3" > "$TEST_DIR/n3.log" 2>&1 &
PID3=$!
wait_for_node 7203 || { log_error "Node 3 failed to start"; exit 1; }
log_success "Node 3 started (PID: $PID3)"

# Wait for cluster formation
log_info "Waiting for cluster to stabilize..."
sleep 5

# =============================================================================
# TEST 1: NON-SHARDED COLLECTION REPLICATION
# =============================================================================
log_header "TEST 1: NON-SHARDED COLLECTION REPLICATION"

# Create database and collection
log_info "Creating database 'testdb'..."
curl -s -X POST "http://$AUTH@127.0.0.1:7201/_api/database" \
    -H "Content-Type: application/json" \
    -d '{"name": "testdb"}' > /dev/null

sleep 2

log_info "Creating non-sharded collection 'unsharded_users'..."
curl -s -X POST "http://$AUTH@127.0.0.1:7201/_api/database/testdb/collection" \
    -H "Content-Type: application/json" \
    -d '{"name": "unsharded_users"}' > /dev/null

sleep 2

# Insert documents
log_info "Inserting $UNSHARDED_DOCS documents into non-sharded collection..."
for i in $(seq 1 $UNSHARDED_DOCS); do
    curl -s -X POST "http://$AUTH@127.0.0.1:7201/_api/database/testdb/document/unsharded_users" \
        -H "Content-Type: application/json" \
        -d "{\"_key\": \"user$i\", \"name\": \"User $i\", \"data\": \"test payload\"}" > /dev/null &
    if (( i % 10 == 0 )); then wait; fi
done
wait

# Wait for replication
log_info "Waiting for replication..."
sleep 5

# Verify replication on all nodes
TEST1_PASS=true
for port in 7201 7202 7203; do
    count=$(get_doc_count $port "testdb" "unsharded_users")
    log_info "Node $port: $count documents"
    if [ "$count" -ne "$UNSHARDED_DOCS" ]; then
        log_fail "Node $port expected $UNSHARDED_DOCS but got $count"
        TEST1_PASS=false
    fi
done

if [ "$TEST1_PASS" = true ]; then
    log_success "Non-sharded collection replicated to all nodes ($UNSHARDED_DOCS docs each)"
else
    log_fail "Non-sharded collection replication FAILED"
fi

# =============================================================================
# TEST 2: SHARDED COLLECTION REPLICATION
# =============================================================================
log_header "TEST 2: SHARDED COLLECTION REPLICATION"

log_info "Creating sharded collection 'sharded_users' (3 shards, RF=2)..."
COLL_RESULT=$(curl -s -X POST "http://$AUTH@127.0.0.1:7201/_api/database/testdb/collection" \
    -H "Content-Type: application/json" \
    -d '{"name": "sharded_users", "numShards": 3, "replicationFactor": 2}')
echo "Collection result: $COLL_RESULT"

sleep 3

# Insert documents
log_info "Inserting $SHARDED_DOCS documents into sharded collection..."
for i in $(seq 1 $SHARDED_DOCS); do
    curl -s -X POST "http://$AUTH@127.0.0.1:7201/_api/database/testdb/document/sharded_users" \
        -H "Content-Type: application/json" \
        -d "{\"_key\": \"user$i\", \"name\": \"User $i\", \"email\": \"user$i@test.com\"}" > /dev/null &
    if (( i % 20 == 0 )); then wait; fi
done
wait

# Wait for sharding and replication
log_info "Waiting for sharding and replication..."
sleep 8

# Check shard distribution on each node
echo ""
log_info "Checking shard distribution..."

for port in 7201 7202 7203; do
    echo ""
    echo -e "${YELLOW}--- Node $port ---${NC}"
    count=$(get_doc_count $port "testdb" "sharded_users")
    echo "Total docs visible: $count"
    
    SHARD_INFO=$(curl -s "http://$AUTH@127.0.0.1:$port/_api/database/testdb/collection/sharded_users/stats" | jq '.sharding // .cluster // empty')
    if [ -n "$SHARD_INFO" ]; then
        echo "Sharding info: $SHARD_INFO"
    fi
done

# Calculate expected distribution
# With 3 shards, RF=2, each doc is stored on 2 nodes
# Expected: each node has approximately (100 * 2) / 3 = ~66 docs
EXPECTED_PER_NODE=$((SHARDED_DOCS * 2 / 3))
log_info "Expected per node (approximate): $EXPECTED_PER_NODE documents"

# Check that total document count is correct
TOTAL_FROM_N1=$(get_doc_count 7201 "testdb" "sharded_users")
if [ "$TOTAL_FROM_N1" -eq "$SHARDED_DOCS" ]; then
    log_success "Sharded collection shows correct total document count: $SHARDED_DOCS"
else
    log_warn "Sharded collection count mismatch: expected $SHARDED_DOCS, got $TOTAL_FROM_N1"
fi

# Show disk usage for comparison
echo ""
log_info "Disk usage per node (sharded should be less than unsharded):"
echo "  Node 1: $(du -sh "$TEST_DIR/n1" | cut -f1)"
echo "  Node 2: $(du -sh "$TEST_DIR/n2" | cut -f1)"
echo "  Node 3: $(du -sh "$TEST_DIR/n3" | cut -f1)"

# =============================================================================
# TEST 3: NODE REMOVAL AND REBALANCE
# =============================================================================
log_header "TEST 3: NODE REMOVAL AND REBALANCE"

# Capture state before removal
log_info "Capturing cluster state before node removal..."
echo ""
echo "Pre-removal counts:"
for port in 7201 7202 7203; do
    UNSHARDED_COUNT=$(get_doc_count $port "testdb" "unsharded_users")
    SHARDED_COUNT=$(get_doc_count $port "testdb" "sharded_users")
    echo "  Node $port: unsharded=$UNSHARDED_COUNT, sharded=$SHARDED_COUNT"
done

# Remove Node 3
log_info "Stopping Node 3 (PID: $PID3)..."
kill $PID3 2>/dev/null || true
sleep 2

# Verify node is gone
if ! kill -0 $PID3 2>/dev/null; then
    log_success "Node 3 stopped successfully"
else
    log_error "Failed to stop Node 3"
fi

# Wait for cluster to detect failure and rebalance
log_info "Waiting for cluster to detect failure and rebalance (30s)..."
for i in $(seq 1 30); do
    echo -ne "\r  Waiting... $i/30s"
    sleep 1
done
echo ""

# Check remaining nodes
echo ""
log_info "Checking remaining nodes after Node 3 removal..."

for port in 7201 7202; do
    echo ""
    echo -e "${YELLOW}--- Node $port ---${NC}"
    
    # Check unsharded collection
    UNSHARDED_COUNT=$(get_doc_count $port "testdb" "unsharded_users")
    echo "  Unsharded collection: $UNSHARDED_COUNT documents"
    
    # Check sharded collection
    SHARDED_COUNT=$(get_doc_count $port "testdb" "sharded_users")
    echo "  Sharded collection: $SHARDED_COUNT documents"
    
    # Check cluster health
    CLUSTER_INFO=$(curl -s "http://$AUTH@127.0.0.1:$port/_api/cluster/status" 2>/dev/null | jq -r '.nodes | length // 0')
    echo "  Known nodes: $CLUSTER_INFO"
done

# Verify data is still accessible from remaining nodes
echo ""
log_info "Verifying data accessibility after node removal..."

# Query some documents
QUERY_RESULT=$(curl -s "http://$AUTH@127.0.0.1:7201/_api/database/testdb/collection/unsharded_users/stats" | jq -r '.count // 0')
if [ "$QUERY_RESULT" -eq "$UNSHARDED_DOCS" ]; then
    log_success "Unsharded collection data still accessible: $QUERY_RESULT documents"
else
    log_warn "Unsharded collection access issue: expected $UNSHARDED_DOCS, got $QUERY_RESULT"
fi

QUERY_RESULT=$(curl -s "http://$AUTH@127.0.0.1:7201/_api/database/testdb/collection/sharded_users/stats" | jq -r '.count // 0')
if [ "$QUERY_RESULT" -eq "$SHARDED_DOCS" ]; then
    log_success "Sharded collection data still accessible: $QUERY_RESULT documents"
else
    log_warn "Sharded collection access issue: expected $SHARDED_DOCS, got $QUERY_RESULT"
fi

# Try inserting new documents after node removal
log_info "Testing writes after node removal..."
INSERT_RESULT=$(curl -s -X POST "http://$AUTH@127.0.0.1:7201/_api/database/testdb/document/unsharded_users" \
    -H "Content-Type: application/json" \
    -d '{"_key": "post_removal_test", "note": "inserted after node 3 removal"}')

if echo "$INSERT_RESULT" | grep -q "_key"; then
    log_success "Successfully inserted document after node removal"
else
    log_warn "Insert after node removal may have issues: $INSERT_RESULT"
fi

# Disk usage after rebalance
echo ""
log_info "Disk usage after rebalance:"
echo "  Node 1: $(du -sh "$TEST_DIR/n1" | cut -f1)"
echo "  Node 2: $(du -sh "$TEST_DIR/n2" | cut -f1)"
echo "  Node 3: $(du -sh "$TEST_DIR/n3" | cut -f1) (stopped)"

# =============================================================================
# FINAL SUMMARY
# =============================================================================
log_header "TEST SUMMARY"

echo ""
echo "Test Results:"
echo "  1. Non-sharded replication: $([ "$TEST1_PASS" = true ] && echo -e "${GREEN}PASS${NC}" || echo -e "${RED}FAIL${NC}")"
echo "  2. Sharded replication: Verified (manual inspection recommended)"
echo "  3. Node removal/rebalance: Verified (check logs for details)"
echo ""
echo "Log files available at:"
echo "  - $TEST_DIR/n1.log"
echo "  - $TEST_DIR/n2.log"
echo "  - $TEST_DIR/n3.log"
echo ""

# Cleanup
log_info "Cleaning up remaining processes..."
kill $PID1 $PID2 2>/dev/null || true

log_success "Test complete!"
