require_relative 'lib/solidb/client'
require 'time'

def run_parallel_benchmark
  port = (ENV['SOLIDB_PORT'] || '9998').to_i
  password = ENV['SOLIDB_PASSWORD'] || 'password'
  
  num_workers = 16
  total_inserts = 10000
  inserts_per_worker = total_inserts / num_workers
  
  db = 'bench_db'
  col = 'ruby_parallel_bench'
  
  # Setup: create database and collection
  setup_client = SoliDB::Client.new('127.0.0.1', port)
  setup_client.connect
  setup_client.auth('_system', 'admin', password)
  
  begin
    setup_client.create_database(db)
  rescue => e
  end
  
  begin
    setup_client.create_collection(db, col)
  rescue => e
  end
  setup_client.close
  
  pids = []
  start_time = Time.now
  
  # Spawn worker processes (using fork to bypass GIL)
  num_workers.times do |worker_id|
    pids << fork do
      client = SoliDB::Client.new('127.0.0.1', port)
      client.connect
      client.auth('_system', 'admin', password)
      
      inserts_per_worker.times do |i|
        client.insert(db, col, {
          worker: worker_id,
          id: i,
          data: "parallel benchmark data"
        })
      end
      
      client.close
    end
  end
  
  # Wait for all processes
  pids.each { |pid| Process.wait(pid) }
  
  end_time = Time.now
  duration = end_time - start_time
  ops_per_sec = total_inserts / duration
  
  puts "RUBY_PARALLEL_BENCH_RESULT:#{ops_per_sec.round(2)}"
end

run_parallel_benchmark
