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
