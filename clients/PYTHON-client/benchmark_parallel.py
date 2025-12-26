import time
import sys
import os
from multiprocessing import Process, Queue

sys.path.append(os.path.dirname(os.path.abspath(__file__)))
from solidb import Client

def worker(worker_id, port, password, num_inserts, result_queue):
    """Each worker creates its own connection and inserts documents"""
    try:
        client = Client("127.0.0.1", port)
        client.connect()
        client.auth("_system", "admin", password)
        
        db = "bench_db"
        col = "python_parallel_bench"
        
        for i in range(num_inserts):
            client.insert(db, col, {
                "worker": worker_id,
                "id": i,
                "data": "parallel benchmark data"
            })
        
        client.close()
        result_queue.put(num_inserts)
    except Exception as e:
        print(f"Worker {worker_id} error: {e}")
        result_queue.put(0)

def run_parallel_benchmark():
    port = int(os.environ.get("SOLIDB_PORT", "9998"))
    password = os.environ.get("SOLIDB_PASSWORD", "password")
    
    num_workers = 16
    total_inserts = 10000
    inserts_per_worker = total_inserts // num_workers
    
    # Setup: create database and collection
    setup_client = Client("127.0.0.1", port)
    setup_client.connect()
    setup_client.auth("_system", "admin", password)
    
    try:
        setup_client.create_database("bench_db")
    except:
        pass
    try:
        setup_client.create_collection("bench_db", "python_parallel_bench")
    except:
        pass
    setup_client.close()
    
    result_queue = Queue()
    processes = []
    
    start_time = time.time()
    
    # Spawn workers
    for w in range(num_workers):
        p = Process(target=worker, args=(w, port, password, inserts_per_worker, result_queue))
        processes.append(p)
        p.start()
    
    # Wait for all workers
    for p in processes:
        p.join()
    
    end_time = time.time()
    
    # Collect results
    total_completed = 0
    while not result_queue.empty():
        total_completed += result_queue.get()
    
    duration = end_time - start_time
    ops_per_sec = total_completed / duration
    
    print(f"PYTHON_PARALLEL_BENCH_RESULT:{ops_per_sec:.2f}")

if __name__ == "__main__":
    run_parallel_benchmark()
