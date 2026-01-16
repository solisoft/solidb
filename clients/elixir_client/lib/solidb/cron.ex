defmodule SoliDB.Cron do
  @moduledoc """
  Cron job management API for SoliDB.
  """

  alias SoliDB.Client

  def list(client) do
    Client.send_command(client, "list_cron_jobs", %{database: Client.get_database(client)})
  end

  def create(client, name, schedule, script_path, opts \\ []) do
    args = %{database: Client.get_database(client), name: name, schedule: schedule, script_path: script_path}
    args = if opts[:params], do: Map.put(args, :params, opts[:params]), else: args
    args = if Keyword.has_key?(opts, :enabled), do: Map.put(args, :enabled, opts[:enabled]), else: args
    args = if opts[:description], do: Map.put(args, :description, opts[:description]), else: args
    Client.send_command(client, "create_cron_job", args)
  end

  def get(client, cron_id) do
    Client.send_command(client, "get_cron_job", %{database: Client.get_database(client), cron_id: cron_id})
  end

  def update(client, cron_id, updates) do
    Client.send_command(client, "update_cron_job", %{database: Client.get_database(client), cron_id: cron_id, updates: updates})
  end

  def delete(client, cron_id) do
    Client.send_command(client, "delete_cron_job", %{database: Client.get_database(client), cron_id: cron_id})
  end

  def toggle(client, cron_id, enabled) do
    Client.send_command(client, "toggle_cron_job", %{database: Client.get_database(client), cron_id: cron_id, enabled: enabled})
  end
end
