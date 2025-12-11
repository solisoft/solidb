#!/bin/bash
BIN=./target/debug/solidb

# Start Node 1
echo "Starting Node 1..."
$BIN --port 8001 --replication-port 9001 --data-dir ./tmp/n1 > tmp/n1-restart.log 2>&1 &
PID1=$!
sleep 2

# Start Node 2
echo "Starting Node 2..."
$BIN --port 8002 --replication-port 9002 --peer 127.0.0.1:9001 --data-dir ./tmp/n2 > tmp/n2-restart.log 2>&1 &
PID2=$!
sleep 2

# Start Node 3
echo "Starting Node 3..."
$BIN --port 8003 --replication-port 9003 --peer 127.0.0.1:9001 --data-dir ./tmp/n3 > tmp/n3-restart.log 2>&1 &
PID3=$!
sleep 5

echo "Cluster restarted. PIDs: $PID1, $PID2, $PID3"
# Keep running until user kills
wait
