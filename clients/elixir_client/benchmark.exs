defmodule SoliDB.Benchmark do
  def run do
    {:ok, client} = SoliDB.Client.connect()
    :ok = SoliDB.Client.auth(client, "_system", "admin", "password")

    db = "bench_db"
    col = "elixir_bench"

    SoliDB.Client.query(client, "_system", "CREATE DATABASE #{db}")
    SoliDB.Client.query(client, db, "CREATE COLLECTION #{col}")

    iterations = 1000
    
    start_time = System.monotonic_time()
    
    for i <- 1..iterations do
      SoliDB.Client.insert(client, db, col, %{id: i, data: "benchmark data content"})
    end
    
    end_time = System.monotonic_time()
    duration = System.convert_time_unit(end_time - start_time, :native, :microsecond) / 1_000_000
    
    ops_per_sec = iterations / duration
    
    IO.puts("ELIXIR_BENCH_RESULT:#{:erlang.float_to_binary(ops_per_sec, [decimals: 2])}")
  end
end

SoliDB.Benchmark.run()
