import time
import sys
import os

# Add parent dir to path to find solidb
sys.path.append(os.path.dirname(os.path.abspath(__file__)))
from solidb import Client

def run_benchmark():
    client = Client("127.0.0.1", 9998)
    client.connect()
    client.auth("_system", "admin", "bench")
    
    db = "bench_db"
    col = "python_bench"
    
    try:
        client.create_database(db)
    except: pass
    
    try:
        client.create_collection(db, col)
    except: pass

    iterations = 1000
    
    start_time = time.time()
    for i in range(iterations):
        client.insert(db, col, {"id": i, "data": "benchmark data content"})
    end_time = time.time()
    
    duration = end_time - start_time
    ops_per_sec = iterations / duration
    
    print(f"PYTHON_BENCH_RESULT:{ops_per_sec:.2f}")
    client.close()

if __name__ == "__main__":
    run_benchmark()
