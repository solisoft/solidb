defmodule SoliDB.Users do
  @moduledoc """
  User management API for SoliDB.
  """

  alias SoliDB.Client

  def list(client) do
    Client.send_command(client, "list_users", %{})
  end

  def create(client, username, password, opts \\ []) do
    args = %{username: username, password: password}
    args = if opts[:roles], do: Map.put(args, :roles, opts[:roles]), else: args
    Client.send_command(client, "create_user", args)
  end

  def get(client, username) do
    Client.send_command(client, "get_user", %{username: username})
  end

  def delete(client, username) do
    Client.send_command(client, "delete_user", %{username: username})
  end

  def get_roles(client, username) do
    Client.send_command(client, "get_user_roles", %{username: username})
  end

  def assign_role(client, username, role, opts \\ []) do
    args = %{username: username, role: role}
    args = if opts[:database], do: Map.put(args, :database, opts[:database]), else: args
    Client.send_command(client, "assign_role", args)
  end

  def revoke_role(client, username, role, opts \\ []) do
    args = %{username: username, role: role}
    args = if opts[:database], do: Map.put(args, :database, opts[:database]), else: args
    Client.send_command(client, "revoke_role", args)
  end

  def me(client) do
    Client.send_command(client, "get_current_user", %{})
  end

  def my_permissions(client) do
    Client.send_command(client, "get_my_permissions", %{})
  end

  def change_password(client, username, old_password, new_password) do
    Client.send_command(client, "change_password", %{
      username: username,
      old_password: old_password,
      new_password: new_password
    })
  end
end
