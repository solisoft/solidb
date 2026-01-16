defmodule SoliDB.TTL do
  @moduledoc """
  TTL (time-to-live) index management API for SoliDB.
  """

  alias SoliDB.Client

  def create_index(client, collection, name, field, expire_after_seconds) do
    Client.send_command(client, "create_ttl_index", %{
      database: Client.get_database(client),
      collection: collection,
      name: name,
      field: field,
      expire_after_seconds: expire_after_seconds
    })
  end

  def list_indexes(client, collection) do
    Client.send_command(client, "list_ttl_indexes", %{database: Client.get_database(client), collection: collection})
  end

  def delete_index(client, collection, index_name) do
    Client.send_command(client, "delete_ttl_index", %{database: Client.get_database(client), collection: collection, index_name: index_name})
  end

  def update_expiration(client, collection, index_name, expire_after_seconds) do
    Client.send_command(client, "update_ttl_expiration", %{
      database: Client.get_database(client),
      collection: collection,
      index_name: index_name,
      expire_after_seconds: expire_after_seconds
    })
  end

  def get_index_info(client, collection, index_name) do
    Client.send_command(client, "ttl_index_info", %{database: Client.get_database(client), collection: collection, index_name: index_name})
  end

  def run_cleanup(client, collection) do
    Client.send_command(client, "ttl_run_cleanup", %{database: Client.get_database(client), collection: collection})
  end
end
