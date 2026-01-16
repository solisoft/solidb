defmodule SoliDB.Env do
  @moduledoc """
  Environment variables management API for SoliDB.
  """

  alias SoliDB.Client

  def list(client) do
    Client.send_command(client, "list_env_vars", %{database: Client.get_database(client)})
  end

  def get(client, key) do
    Client.send_command(client, "get_env_var", %{database: Client.get_database(client), key: key})
  end

  def set(client, key, value) do
    Client.send_command(client, "set_env_var", %{database: Client.get_database(client), key: key, value: value})
  end

  def delete(client, key) do
    Client.send_command(client, "delete_env_var", %{database: Client.get_database(client), key: key})
  end

  def set_bulk(client, vars) do
    Client.send_command(client, "set_env_vars_bulk", %{database: Client.get_database(client), vars: vars})
  end
end
