defmodule SoliDB.CollectionsOps do
  @moduledoc """
  Advanced collection operations API for SoliDB.
  """

  alias SoliDB.Client

  def truncate(client, collection) do
    Client.send_command(client, "truncate_collection", %{database: Client.get_database(client), collection: collection})
  end

  def compact(client, collection) do
    Client.send_command(client, "compact_collection", %{database: Client.get_database(client), collection: collection})
  end

  def stats(client, collection) do
    Client.send_command(client, "collection_stats", %{database: Client.get_database(client), collection: collection})
  end

  def prune(client, collection, opts \\ []) do
    args = %{database: Client.get_database(client), collection: collection}
    args = if opts[:older_than], do: Map.put(args, :older_than, opts[:older_than]), else: args
    args = if opts[:field], do: Map.put(args, :field, opts[:field]), else: args
    Client.send_command(client, "prune_collection", args)
  end

  def recount(client, collection) do
    Client.send_command(client, "recount_collection", %{database: Client.get_database(client), collection: collection})
  end

  def repair(client, collection) do
    Client.send_command(client, "repair_collection", %{database: Client.get_database(client), collection: collection})
  end

  def set_schema(client, collection, schema) do
    Client.send_command(client, "set_collection_schema", %{database: Client.get_database(client), collection: collection, schema: schema})
  end

  def get_schema(client, collection) do
    Client.send_command(client, "get_collection_schema", %{database: Client.get_database(client), collection: collection})
  end

  def delete_schema(client, collection) do
    Client.send_command(client, "delete_collection_schema", %{database: Client.get_database(client), collection: collection})
  end

  def export(client, collection, format) do
    Client.send_command(client, "export_collection", %{database: Client.get_database(client), collection: collection, format: format})
  end

  def import(client, collection, data, format) do
    Client.send_command(client, "import_collection", %{database: Client.get_database(client), collection: collection, data: data, format: format})
  end

  def get_sharding(client, collection) do
    Client.send_command(client, "get_collection_sharding", %{database: Client.get_database(client), collection: collection})
  end

  def set_sharding(client, collection, config) do
    args = Map.merge(%{database: Client.get_database(client), collection: collection}, config)
    Client.send_command(client, "set_collection_sharding", args)
  end
end
