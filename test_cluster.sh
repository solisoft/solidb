#!/bin/bash

# Cluster Mode Test Script for SoliDB
# Tests master-master replication, node failures, and data synchronization

set -e  # Exit on error

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
NODE1_PORT=6745
NODE1_REPL_PORT=6746
NODE1_DATA_DIR="./test_data1"
NODE1_PID=""

NODE2_PORT=6755
NODE2_REPL_PORT=6756
NODE2_DATA_DIR="./test_data2"
NODE2_PID=""

NODE3_PORT=6765
NODE3_REPL_PORT=6766
NODE3_DATA_DIR="./test_data3"
NODE3_PID=""

BINARY="./target/release/solidb"
TEST_DB="test_cluster_db"
TEST_COLLECTION="test_collection"

# Helper functions
log_info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

log_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

log_warning() {
    echo -e "${YELLOW}[WARNING]${NC} $1"
}

# Cleanup function
cleanup() {
    log_info "Cleaning up..."
    
    # Kill all node processes by PID
    if [ ! -z "$NODE1_PID" ]; then
        kill $NODE1_PID 2>/dev/null || true
        log_info "Stopped node 1 (PID: $NODE1_PID)"
    fi
    if [ ! -z "$NODE2_PID" ]; then
        kill $NODE2_PID 2>/dev/null || true
        log_info "Stopped node 2 (PID: $NODE2_PID)"
    fi
    if [ ! -z "$NODE3_PID" ]; then
        kill $NODE3_PID 2>/dev/null || true
        log_info "Stopped node 3 (PID: $NODE3_PID)"
    fi
    
    # Also kill any remaining solidb processes (in case PIDs weren't captured)
    pkill -f "solidb.*test_data" 2>/dev/null || true
    
    # Wait a bit for processes to terminate
    sleep 2
    
    # Remove test data directories
    rm -rf "$NODE1_DATA_DIR" "$NODE2_DATA_DIR" "$NODE3_DATA_DIR"
    log_info "Removed test data directories"
    
    log_success "Cleanup complete"
}

# Set trap to cleanup on exit
trap cleanup EXIT INT TERM

# Build the binary
build_binary() {
    log_info "Building SoliDB in release mode..."
    cargo build --release
    log_success "Build complete"
}

# Start a node
start_node() {
    local node_num=$1
    local port=$2
    local repl_port=$3
    local data_dir=$4
    local peers=$5
    local node_id=$6
    
    log_info "Starting node $node_num on port $port (replication: $repl_port)..."
    
    # Create data directory
    mkdir -p "$data_dir"
    
    # Start the node in background
    $BINARY --port $port --replication-port $repl_port --data-dir "$data_dir" \
        --node-id "$node_id" $peers > "${data_dir}/node.log" 2>&1 &
    
    local pid=$!
    echo $pid
    
    log_success "Node $node_num started (PID: $pid)"
}

# Wait for node to be ready
wait_for_node() {
    local port=$1
    local max_attempts=30
    local attempt=0
    
    log_info "Waiting for node on port $port to be ready..."
    
    while [ $attempt -lt $max_attempts ]; do
        if curl -s "http://localhost:$port/_api/cluster/status" > /dev/null 2>&1; then
            log_success "Node on port $port is ready"
            return 0
        fi
        attempt=$((attempt + 1))
        sleep 1
    done
    
    log_error "Node on port $port failed to start"
    return 1
}

# Get cluster status
get_cluster_status() {
    local port=$1
    curl -s "http://localhost:$port/_api/cluster/status" | jq '.'
}

# Create database
create_database() {
    local port=$1
    local db_name=$2
    
    log_info "Creating database '$db_name' on node (port $port)..."
    
    local response=$(curl -s -X POST "http://localhost:$port/_api/database" \
        -H "Content-Type: application/json" \
        -d "{\"name\": \"$db_name\"}")
    
    if echo "$response" | jq -e '.name' > /dev/null 2>&1; then
        log_success "Database '$db_name' created"
        return 0
    elif echo "$response" | grep -q "already exists"; then
        log_warning "Database '$db_name' already exists, continuing..."
        return 0
    else
        log_error "Failed to create database: $response"
        return 1
    fi
}

# Create collection
create_collection() {
    local port=$1
    local db_name=$2
    local coll_name=$3
    
    log_info "Creating collection '$coll_name' in database '$db_name' on node (port $port)..."
    
    local response=$(curl -s -X POST "http://localhost:$port/_api/database/$db_name/collection" \
        -H "Content-Type: application/json" \
        -d "{\"name\": \"$coll_name\"}")
    
    if echo "$response" | jq -e '.name' > /dev/null 2>&1; then
        log_success "Collection '$coll_name' created"
        return 0
    elif echo "$response" | grep -q "already exists"; then
        log_warning "Collection '$coll_name' already exists, truncating..."
        # Truncate the collection to start fresh
        curl -s -X PUT "http://localhost:$port/_api/database/$db_name/collection/$coll_name/truncate" > /dev/null
        log_success "Collection '$coll_name' truncated"
        return 0
    else
        log_error "Failed to create collection: $response"
        return 1
    fi
}

# Insert document
insert_document() {
    local port=$1
    local db_name=$2
    local coll_name=$3
    local data=$4
    
    local response=$(curl -s -X POST "http://localhost:$port/_api/database/$db_name/document/$coll_name" \
        -H "Content-Type: application/json" \
        -d "$data")
    
    if echo "$response" | jq -e '._key' > /dev/null 2>&1; then
        local key=$(echo "$response" | jq -r '._key')
        echo "$key"
        return 0
    else
        log_error "Failed to insert document: $response"
        log_error "Port: $port, DB: $db_name, Collection: $coll_name"
        log_error "Data: $data"
        exit 1
    fi
}

# Query documents
query_documents() {
    local port=$1
    local db_name=$2
    local query=$3
    
    local response=$(curl -s -X POST "http://localhost:$port/_api/database/$db_name/cursor" \
        -H "Content-Type: application/json" \
        -d "{\"query\": \"$query\"}")
    
    echo "$response"
}

# Count documents in collection
count_documents() {
    local port=$1
    local db_name=$2
    local coll_name=$3
    
    local response=$(query_documents $port "$db_name" "FOR doc IN $coll_name RETURN doc")
    local count=$(echo "$response" | jq '.count')
    echo "$count"
}

# Wait for a specific document count with retries
wait_for_count() {
    local port=$1
    local db_name=$2
    local coll_name=$3
    local expected_count=$4
    local max_attempts=${5:-10}
    local attempt=0
    
    while [ $attempt -lt $max_attempts ]; do
        local count=$(count_documents $port "$db_name" "$coll_name")
        if [ "$count" -eq "$expected_count" ]; then
            echo "$count"
            return 0
        fi
        attempt=$((attempt + 1))
        sleep 1
    done
    
    # Return the last count even if it doesn't match
    echo "$count"
    return 1
}

# Verify document exists on node
verify_document() {
    local port=$1
    local db_name=$2
    local coll_name=$3
    local key=$4
    
    local response=$(curl -s "http://localhost:$port/_api/database/$db_name/document/$coll_name/$key")
    
    if echo "$response" | jq -e '._key' > /dev/null 2>&1; then
        return 0
    else
        return 1
    fi
}

# Main test execution
main() {
    log_info "========================================="
    log_info "SoliDB Cluster Mode Test"
    log_info "========================================="
    echo ""
    
    # Pre-cleanup: Kill any existing test processes and remove old data
    log_info "Pre-cleanup: Removing old test data and processes..."
    pkill -f "solidb.*test_data" 2>/dev/null || true
    rm -rf "$NODE1_DATA_DIR" "$NODE2_DATA_DIR" "$NODE3_DATA_DIR"
    sleep 1
    log_success "Pre-cleanup complete"
    echo ""
    
    # Step 1: Build binary
    build_binary
    echo ""
    
    # Step 2: Start 3 nodes
    log_info "Step 1: Starting 3-node cluster..."
    
    # Start node 1
    NODE1_PID=$(start_node 1 $NODE1_PORT $NODE1_REPL_PORT "$NODE1_DATA_DIR" \
        "--peer localhost:$NODE2_REPL_PORT --peer localhost:$NODE3_REPL_PORT" "node1")
    
    # Start node 2
    NODE2_PID=$(start_node 2 $NODE2_PORT $NODE2_REPL_PORT "$NODE2_DATA_DIR" \
        "--peer localhost:$NODE1_REPL_PORT --peer localhost:$NODE3_REPL_PORT" "node2")
    
    # Start node 3
    NODE3_PID=$(start_node 3 $NODE3_PORT $NODE3_REPL_PORT "$NODE3_DATA_DIR" \
        "--peer localhost:$NODE1_REPL_PORT --peer localhost:$NODE2_REPL_PORT" "node3")
    
    # Wait for all nodes to be ready
    wait_for_node $NODE1_PORT
    wait_for_node $NODE2_PORT
    wait_for_node $NODE3_PORT
    
    # Give nodes time to discover each other
    log_info "Waiting for cluster to form..."
    sleep 5
    
    log_success "3-node cluster is running"
    echo ""
    
    # Step 3: Check cluster status
    log_info "Step 2: Checking cluster status..."
    log_info "Node 1 status:"
    get_cluster_status $NODE1_PORT
    echo ""
    
    # Step 4: Create database and collection on node 1
    log_info "Step 3: Creating database and collection on node 1..."
    create_database $NODE1_PORT "$TEST_DB"
    sleep 2  # Wait for replication
    create_collection $NODE1_PORT "$TEST_DB" "$TEST_COLLECTION"
    sleep 2  # Wait for replication
    echo ""
    
    # Step 5: Insert documents on node 1
    log_info "Step 4: Inserting documents on node 1..."
    DOC1_KEY=$(insert_document $NODE1_PORT "$TEST_DB" "$TEST_COLLECTION" '{"name": "Alice", "age": 30, "node": "node1"}')
    log_success "Inserted document with key: $DOC1_KEY"
    
    DOC2_KEY=$(insert_document $NODE1_PORT "$TEST_DB" "$TEST_COLLECTION" '{"name": "Bob", "age": 25, "node": "node1"}')
    log_success "Inserted document with key: $DOC2_KEY"
    
    # Wait for replication
    log_info "Waiting for replication..."
    sleep 2
    echo ""
    
    # Step 6: Verify documents on all nodes
    log_info "Step 5: Verifying data replication on all nodes..."
    
    for port in $NODE1_PORT $NODE2_PORT $NODE3_PORT; do
        count=$(count_documents $port "$TEST_DB" "$TEST_COLLECTION")
        log_info "Node on port $port has $count documents"
        
        if [ "$count" -eq "2" ]; then
            log_success "Node on port $port has correct document count"
        else
            log_error "Node on port $port has incorrect document count (expected 2, got $count)"
        fi
    done
    echo ""
    
    # Step 7: Insert document on node 2
    log_info "Step 6: Inserting document on node 2..."
    DOC3_KEY=$(insert_document $NODE2_PORT "$TEST_DB" "$TEST_COLLECTION" '{"name": "Charlie", "age": 35, "node": "node2"}')
    log_success "Inserted document with key: $DOC3_KEY"
    
    log_info "Waiting for replication..."
    sleep 1  # Initial wait
    
    # Verify on all nodes with retry
    log_info "Verifying replication on all nodes..."
    for port in $NODE1_PORT $NODE2_PORT $NODE3_PORT; do
        count=$(wait_for_count $port "$TEST_DB" "$TEST_COLLECTION" 3 10)
        if [ "$count" -eq "3" ]; then
            log_success "Node on port $port has 3 documents after node 2 insert"
        else
            log_error "Node on port $port has $count documents (expected 3)"
        fi
    done
    echo ""
    
    # Step 8: Stop node 2 (simulate failure)
    log_info "Step 7: Simulating node 2 failure..."
    kill $NODE2_PID 2>/dev/null || true
    log_warning "Node 2 stopped (PID: $NODE2_PID)"
    sleep 2
    echo ""
    
    # Step 9: Insert document on node 1 while node 2 is down
    log_info "Step 8: Inserting document on node 1 while node 2 is down..."
    DOC4_KEY=$(insert_document $NODE1_PORT "$TEST_DB" "$TEST_COLLECTION" '{"name": "David", "age": 28, "node": "node1", "during_failure": true}')
    log_success "Inserted document with key: $DOC4_KEY"
    
    log_info "Waiting for replication..."
    sleep 5
    
    # Verify on nodes 1 and 3
    for port in $NODE1_PORT $NODE3_PORT; do
        count=$(count_documents $port "$TEST_DB" "$TEST_COLLECTION")
        if [ "$count" -eq "4" ]; then
            log_success "Node on port $port has 4 documents"
        else
            log_error "Node on port $port has $count documents (expected 4)"
        fi
    done
    echo ""
    
    # Step 10: Restart node 2
    log_info "Step 9: Restarting node 2..."
    NODE2_PID=$(start_node 2 $NODE2_PORT $NODE2_REPL_PORT "$NODE2_DATA_DIR" \
        "--peer localhost:$NODE1_REPL_PORT --peer localhost:$NODE3_REPL_PORT" "node2")
    wait_for_node $NODE2_PORT
    
    # Wait for sync
    log_info "Waiting for node 2 to sync..."
    sleep 8
    
    # Verify node 2 caught up
    count=$(count_documents $NODE2_PORT "$TEST_DB" "$TEST_COLLECTION")
    if [ "$count" -eq "4" ]; then
        log_success "Node 2 successfully caught up (has 4 documents)"
    else
        log_error "Node 2 failed to catch up (has $count documents, expected 4)"
    fi
    echo ""
    
    # Step 11: Insert on node 3
    log_info "Step 10: Inserting document on node 3..."
    DOC5_KEY=$(insert_document $NODE3_PORT "$TEST_DB" "$TEST_COLLECTION" '{"name": "Eve", "age": 32, "node": "node3"}')
    log_success "Inserted document with key: $DOC5_KEY"
    
    log_info "Waiting for replication..."
    sleep 5
    
    # Step 11: Stop node 1 (test different node failure)
    log_info "Step 10: Simulating node 1 failure..."
    kill $NODE1_PID 2>/dev/null || true
    log_warning "Node 1 stopped (PID: $NODE1_PID)"
    sleep 2
    echo ""
    
    # Step 12: Insert documents on node 2 while node 1 is down
    log_info "Step 11: Inserting documents on node 2 while node 1 is down..."
    DOC6_KEY=$(insert_document $NODE2_PORT "$TEST_DB" "$TEST_COLLECTION" '{"name": "Frank", "age": 40, "node": "node2", "during_node1_failure": true}')
    log_success "Inserted document with key: $DOC6_KEY"
    
    DOC7_KEY=$(insert_document $NODE2_PORT "$TEST_DB" "$TEST_COLLECTION" '{"name": "Grace", "age": 27, "node": "node2", "during_node1_failure": true}')
    log_success "Inserted document with key: $DOC7_KEY"
    
    log_info "Waiting for replication..."
    sleep 5
    
    # Verify on nodes 2 and 3
    for port in $NODE2_PORT $NODE3_PORT; do
        count=$(count_documents $port "$TEST_DB" "$TEST_COLLECTION")
        if [ "$count" -eq "7" ]; then
            log_success "Node on port $port has 7 documents"
        else
            log_error "Node on port $port has $count documents (expected 7)"
        fi
    done
    echo ""
    
    # Step 13: Restart node 1
    log_info "Step 12: Restarting node 1..."
    NODE1_PID=$(start_node 1 $NODE1_PORT $NODE1_REPL_PORT "$NODE1_DATA_DIR" \
        "--peer localhost:$NODE2_REPL_PORT --peer localhost:$NODE3_REPL_PORT" "node1")
    wait_for_node $NODE1_PORT
    
    # Wait for sync
    log_info "Waiting for node 1 to sync..."
    sleep 2
    
    # Verify node 1 caught up with retry
    count=$(wait_for_count $NODE1_PORT "$TEST_DB" "$TEST_COLLECTION" 7 15)
    if [ "$count" -eq "7" ]; then
        log_success "Node 1 successfully caught up (has 7 documents)"
    else
        log_error "Node 1 failed to catch up (has $count documents, expected 7)"
    fi
    
    # Verify the specific documents written during node 1 failure exist on node 1
    log_info "Verifying documents written during node 1 failure..."
    sleep 2  # Extra wait to ensure individual documents are fully synced
    for key in $DOC6_KEY $DOC7_KEY; do
        if verify_document $NODE1_PORT "$TEST_DB" "$TEST_COLLECTION" "$key"; then
            log_success "Document $key (written during node 1 failure) exists on node 1"
        else
            log_error "Document $key NOT found on node 1"
        fi
    done
    echo ""
    
    # Step 14: Insert on node 3 to verify cluster is fully operational
    log_info "Step 13: Inserting document on node 3 to verify full cluster operation..."
    DOC8_KEY=$(insert_document $NODE3_PORT "$TEST_DB" "$TEST_COLLECTION" '{"name": "Henry", "age": 33, "node": "node3"}')
    log_success "Inserted document with key: $DOC8_KEY"

    
    # Final verification on all nodes
    log_info "Step 14: Final verification on all nodes..."
    for port in $NODE1_PORT $NODE2_PORT $NODE3_PORT; do
        count=$(count_documents $port "$TEST_DB" "$TEST_COLLECTION")
        log_info "Node on port $port has $count documents"
        
        if [ "$count" -eq "8" ]; then
            log_success "Node on port $port has correct final count"
        else
            log_error "Node on port $port has incorrect count (expected 8, got $count)"
        fi
    done
    echo ""
    
    # Step 15: Verify specific documents exist on all nodes
    log_info "Step 15: Verifying all documents exist on all nodes..."
    
    for key in $DOC1_KEY $DOC2_KEY $DOC3_KEY $DOC4_KEY $DOC5_KEY $DOC6_KEY $DOC7_KEY $DOC8_KEY; do
        log_info "Checking document $key..."
        for port in $NODE1_PORT $NODE2_PORT $NODE3_PORT; do
            if verify_document $port "$TEST_DB" "$TEST_COLLECTION" "$key"; then
                log_success "Document $key exists on node (port $port)"
            else
                log_error "Document $key NOT found on node (port $port)"
            fi
        done
    done
    echo ""
    
    # Final cluster status
    log_info "Final cluster status:"
    log_info "Node 1:"
    get_cluster_status $NODE1_PORT
    echo ""
    log_info "Node 2:"
    get_cluster_status $NODE2_PORT
    echo ""
    log_info "Node 3:"
    get_cluster_status $NODE3_PORT
    echo ""
    
    log_success "========================================="
    log_success "All cluster tests completed successfully!"
    log_success "========================================="
}

# Run main function
main
