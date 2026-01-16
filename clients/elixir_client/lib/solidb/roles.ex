defmodule SoliDB.Roles do
  @moduledoc """
  RBAC roles management API for SoliDB.
  """

  alias SoliDB.Client

  def list(client) do
    Client.send_command(client, "list_roles", %{})
  end

  def create(client, name, permissions, opts \\ []) do
    args = %{name: name, permissions: permissions}
    args = if opts[:description], do: Map.put(args, :description, opts[:description]), else: args
    Client.send_command(client, "create_role", args)
  end

  def get(client, name) do
    Client.send_command(client, "get_role", %{role_name: name})
  end

  def update(client, name, permissions, opts \\ []) do
    args = %{role_name: name, permissions: permissions}
    args = if opts[:description], do: Map.put(args, :description, opts[:description]), else: args
    Client.send_command(client, "update_role", args)
  end

  def delete(client, name) do
    Client.send_command(client, "delete_role", %{role_name: name})
  end
end
