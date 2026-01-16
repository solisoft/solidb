defmodule SoliDB.ApiKeys do
  @moduledoc """
  API keys management API for SoliDB.
  """

  alias SoliDB.Client

  def list(client) do
    Client.send_command(client, "list_api_keys", %{})
  end

  def create(client, name, permissions, opts \\ []) do
    args = %{name: name, permissions: permissions}
    args = if opts[:expires_at], do: Map.put(args, :expires_at, opts[:expires_at]), else: args
    Client.send_command(client, "create_api_key", args)
  end

  def get(client, key_id) do
    Client.send_command(client, "get_api_key", %{key_id: key_id})
  end

  def delete(client, key_id) do
    Client.send_command(client, "delete_api_key", %{key_id: key_id})
  end

  def regenerate(client, key_id) do
    Client.send_command(client, "regenerate_api_key", %{key_id: key_id})
  end
end
