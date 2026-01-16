defmodule SoliDB.IndexesOps do
  @moduledoc """
  Advanced index operations API for SoliDB.
  """

  alias SoliDB.Client

  def rebuild(client, collection, index_name) do
    Client.send_command(client, "rebuild_index", %{database: Client.get_database(client), collection: collection, index_name: index_name})
  end

  def rebuild_all(client, collection) do
    Client.send_command(client, "rebuild_all_indexes", %{database: Client.get_database(client), collection: collection})
  end

  def hybrid_search(client, collection, query) do
    args = Map.merge(%{database: Client.get_database(client), collection: collection}, query)
    Client.send_command(client, "hybrid_search", args)
  end

  def analyze(client, collection, index_name) do
    Client.send_command(client, "analyze_index", %{database: Client.get_database(client), collection: collection, index_name: index_name})
  end

  def get_usage_stats(client, collection) do
    Client.send_command(client, "index_usage_stats", %{database: Client.get_database(client), collection: collection})
  end
end
