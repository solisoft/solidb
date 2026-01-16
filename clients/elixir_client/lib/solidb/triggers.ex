defmodule SoliDB.Triggers do
  @moduledoc """
  Database triggers management API for SoliDB.
  """

  alias SoliDB.Client

  def list(client) do
    Client.send_command(client, "list_triggers", %{database: Client.get_database(client)})
  end

  def list_by_collection(client, collection) do
    Client.send_command(client, "list_triggers_by_collection", %{database: Client.get_database(client), collection: collection})
  end

  def create(client, name, collection, event, timing, script_path, opts \\ []) do
    args = %{
      database: Client.get_database(client),
      name: name,
      collection: collection,
      event: event,
      timing: timing,
      script_path: script_path
    }
    args = if Keyword.has_key?(opts, :enabled), do: Map.put(args, :enabled, opts[:enabled]), else: args
    Client.send_command(client, "create_trigger", args)
  end

  def get(client, trigger_id) do
    Client.send_command(client, "get_trigger", %{database: Client.get_database(client), trigger_id: trigger_id})
  end

  def update(client, trigger_id, updates) do
    Client.send_command(client, "update_trigger", %{database: Client.get_database(client), trigger_id: trigger_id, updates: updates})
  end

  def delete(client, trigger_id) do
    Client.send_command(client, "delete_trigger", %{database: Client.get_database(client), trigger_id: trigger_id})
  end

  def toggle(client, trigger_id, enabled) do
    Client.send_command(client, "toggle_trigger", %{database: Client.get_database(client), trigger_id: trigger_id, enabled: enabled})
  end
end
