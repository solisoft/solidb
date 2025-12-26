require_relative 'lib/solidb/client'
require 'time'

def run_benchmark
  client = SoliDB::Client.new('127.0.0.1', 9999)
  client.connect
  client.auth('_system', 'admin', 'admin')
  
  db = 'bench_db'
  col = 'ruby_bench'
  
  begin
    client.query('_system', "CREATE DATABASE #{db}")
  rescue
  end
  
  begin
    client.query(db, "CREATE COLLECTION #{col}")
  rescue
  end

  iterations = 1000
  
  start_time = Time.now
  iterations.times do |i|
    client.insert(db, col, { id: i, data: "benchmark data content" })
  end
  end_time = Time.now
  
  duration = end_time - start_time
  ops_per_sec = iterations / duration
  
  puts "RUBY_BENCH_RESULT:#{ops_per_sec.round(2)}"
  client.close
end

run_benchmark
