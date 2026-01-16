defmodule SoliDB.Scripts do
  @moduledoc """
  Scripts management API for SoliDB.
  """

  alias SoliDB.Client

  def create(client, name, path, methods, code, opts \\ []) do
    args = %{
      database: Client.get_database(client),
      name: name,
      path: path,
      methods: methods,
      code: code
    }
    args = if opts[:description], do: Map.put(args, :description, opts[:description]), else: args
    args = if opts[:collection], do: Map.put(args, :collection, opts[:collection]), else: args
    Client.send_command(client, "create_script", args)
  end

  def list(client) do
    Client.send_command(client, "list_scripts", %{database: Client.get_database(client)})
  end

  def get(client, script_id) do
    Client.send_command(client, "get_script", %{database: Client.get_database(client), script_id: script_id})
  end

  def update(client, script_id, updates) do
    Client.send_command(client, "update_script", %{database: Client.get_database(client), script_id: script_id, updates: updates})
  end

  def delete(client, script_id) do
    Client.send_command(client, "delete_script", %{database: Client.get_database(client), script_id: script_id})
  end

  def get_stats(client) do
    Client.send_command(client, "get_script_stats", %{})
  end
end
