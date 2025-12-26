import time
import sys
import os

# Add parent dir to path to find solidb
sys.path.append(os.path.dirname(os.path.abspath(__file__)))
from solidb import Client

def run_benchmark():
    port = int(os.environ.get("SOLIDB_PORT", "9998"))
    password = os.environ.get("SOLIDB_PASSWORD", "password")
    
    client = Client("127.0.0.1", port)
    client.connect()
    client.auth("_system", "admin", password)
    
    db = "bench_db"
    col = "python_bench"
    
    try:
        client.create_database(db)
    except: pass
    
    try:
        client.create_collection(db, col)
    except: pass

    iterations = 1000
    
    # INSERT BENCHMARK
    start_time = time.time()
    for i in range(iterations):
        key = f"bench_{i}"
        client.insert(db, col, {"id": i, "data": "benchmark data content"}, key=key)
    end_time = time.time()
    
    insert_duration = end_time - start_time
    insert_ops_per_sec = iterations / insert_duration
    print(f"PYTHON_BENCH_RESULT:{insert_ops_per_sec:.2f}")
    
    # READ BENCHMARK
    start_time = time.time()
    for i in range(iterations):
        key = f"bench_{i}"
        client.get(db, col, key)
    end_time = time.time()
    
    read_duration = end_time - start_time
    read_ops_per_sec = iterations / read_duration
    print(f"PYTHON_READ_BENCH_RESULT:{read_ops_per_sec:.2f}")
    
    client.close()

if __name__ == "__main__":
    run_benchmark()

