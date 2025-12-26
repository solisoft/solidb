require_relative 'lib/solidb/client'
require 'time'

def run_benchmark
  port = (ENV['SOLIDB_PORT'] || '9998').to_i
  password = ENV['SOLIDB_PASSWORD'] || 'password'
  
  client = SoliDB::Client.new('127.0.0.1', port)
  client.connect
  client.auth('_system', 'admin', password)
  
  db = 'bench_db'
  col = 'ruby_bench'
  
  begin
    client.create_database(db)
  rescue => e
  end
  
  begin
    client.create_collection(db, col)
  rescue => e
  end

  iterations = 1000
  
  # INSERT BENCHMARK
  start_time = Time.now
  iterations.times do |i|
    key = "bench_#{i}"
    client.insert(db, col, { id: i, data: "benchmark data content" }, key)
  end
  end_time = Time.now
  
  insert_duration = end_time - start_time
  insert_ops_per_sec = iterations / insert_duration
  puts "RUBY_BENCH_RESULT:#{insert_ops_per_sec.round(2)}"
  
  # READ BENCHMARK
  start_time = Time.now
  iterations.times do |i|
    key = "bench_#{i}"
    client.get(db, col, key)
  end
  end_time = Time.now
  
  read_duration = end_time - start_time
  read_ops_per_sec = iterations / read_duration
  puts "RUBY_READ_BENCH_RESULT:#{read_ops_per_sec.round(2)}"
  
  client.close
end

run_benchmark
