defmodule SoliDB.Jobs do
  @moduledoc """
  Jobs/Queue management API for SoliDB.
  """

  alias SoliDB.Client

  def list_queues(client) do
    Client.send_command(client, "list_queues", %{database: Client.get_database(client)})
  end

  def list_jobs(client, queue_name, opts \\ []) do
    args = %{database: Client.get_database(client), queue_name: queue_name}
    args = if opts[:status], do: Map.put(args, :status, opts[:status]), else: args
    args = if opts[:limit], do: Map.put(args, :limit, opts[:limit]), else: args
    args = if opts[:offset], do: Map.put(args, :offset, opts[:offset]), else: args
    Client.send_command(client, "list_jobs", args)
  end

  def enqueue(client, queue_name, script_path, opts \\ []) do
    args = %{database: Client.get_database(client), queue_name: queue_name, script_path: script_path}
    args = if opts[:params], do: Map.put(args, :params, opts[:params]), else: args
    args = if opts[:priority], do: Map.put(args, :priority, opts[:priority]), else: args
    args = if opts[:run_at], do: Map.put(args, :run_at, opts[:run_at]), else: args
    Client.send_command(client, "enqueue_job", args)
  end

  def cancel(client, job_id) do
    Client.send_command(client, "cancel_job", %{database: Client.get_database(client), job_id: job_id})
  end

  def get(client, job_id) do
    Client.send_command(client, "get_job", %{database: Client.get_database(client), job_id: job_id})
  end
end
