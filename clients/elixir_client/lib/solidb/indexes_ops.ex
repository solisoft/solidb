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

  # --- Foreign keys ---

  @doc """
  The foreign keys declared on `collection` (`"foreign_keys"`) and those of
  other collections that refer to it (`"referenced_by"`).
  """
  def list_foreign_keys(client, collection) do
    Client.send_command(client, "list_foreign_keys", %{database: Client.get_database(client), collection: collection})
  end

  @doc """
  Declare that `field` of `collection` holds the `_key` (with `by: "id"`,
  the `_id`) of a document of `references`, a collection of the same
  database, or null. Options: `:on_delete` ("restrict", the default,
  "cascade" or "set_null"), `:by`, `:name`. Refused when stored documents
  already refer to a missing parent: `check_foreign_key/5` lists them.
  """
  def create_foreign_key(client, collection, field, references, opts \\ []) do
    args =
      %{database: Client.get_database(client), collection: collection, field: field, references: references}
      |> put_opt(:on_delete, opts[:on_delete])
      |> put_opt(:by, opts[:by])
      |> put_opt(:name, opts[:name])

    Client.send_command(client, "create_foreign_key", args)
  end

  def delete_foreign_key(client, collection, name) do
    Client.send_command(client, "delete_foreign_key", %{database: Client.get_database(client), collection: collection, name: name})
  end

  @doc """
  The documents of `collection` whose `field` names a document that is not
  in `references`: a count and a sample.
  """
  def check_foreign_key(client, collection, field, references, by \\ nil) do
    args =
      %{database: Client.get_database(client), collection: collection, field: field, references: references}
      |> put_opt(:by, by)

    Client.send_command(client, "check_foreign_key", args)
  end

  defp put_opt(args, _key, nil), do: args
  defp put_opt(args, key, value), do: Map.put(args, key, value)
end
