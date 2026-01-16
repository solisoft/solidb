defmodule SoliDB.Vector do
  @moduledoc """
  Vector index and search API for SoliDB.
  """

  alias SoliDB.Client

  def create_index(client, collection, name, field, dimensions, opts \\ []) do
    args = %{database: Client.get_database(client), collection: collection, name: name, field: field, dimensions: dimensions}
    args = if opts[:metric], do: Map.put(args, :metric, opts[:metric]), else: args
    args = Enum.reduce(Keyword.delete(Keyword.delete(opts, :metric), :dimensions), args, fn {k, v}, acc -> Map.put(acc, k, v) end)
    Client.send_command(client, "create_vector_index", args)
  end

  def list_indexes(client, collection) do
    Client.send_command(client, "list_vector_indexes", %{database: Client.get_database(client), collection: collection})
  end

  def delete_index(client, collection, index_name) do
    Client.send_command(client, "delete_vector_index", %{database: Client.get_database(client), collection: collection, index_name: index_name})
  end

  def search(client, collection, vector, limit, opts \\ []) do
    args = %{database: Client.get_database(client), collection: collection, vector: vector, limit: limit}
    args = if opts[:filter], do: Map.put(args, :filter, opts[:filter]), else: args
    Client.send_command(client, "vector_search", args)
  end

  def search_by_document(client, collection, doc_key, field, limit, opts \\ []) do
    args = %{database: Client.get_database(client), collection: collection, doc_key: doc_key, field: field, limit: limit}
    args = if opts[:filter], do: Map.put(args, :filter, opts[:filter]), else: args
    Client.send_command(client, "vector_search_by_doc", args)
  end

  def quantize(client, collection, index_name, quantization) do
    Client.send_command(client, "vector_quantize", %{database: Client.get_database(client), collection: collection, index_name: index_name, quantization: quantization})
  end

  def dequantize(client, collection, index_name) do
    Client.send_command(client, "vector_dequantize", %{database: Client.get_database(client), collection: collection, index_name: index_name})
  end

  def get_index_info(client, collection, index_name) do
    Client.send_command(client, "vector_index_info", %{database: Client.get_database(client), collection: collection, index_name: index_name})
  end
end
