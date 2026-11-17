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

  # --- Field rules ---

  @doc """
  The field rules of `collection`, in the order they were declared.
  """
  def get_field_rules(client, collection) do
    Client.send_command(client, "get_field_rules", %{database: Client.get_database(client), collection: collection})
  end

  @doc """
  Declare what `collection` asks of top-level fields of its documents, in
  place of the rules it had; an empty list removes them. A rule is a map
  with `field` and any of `required`, `type` ("string", "number",
  "integer", "bool", "object", "array"), `min`, `max` and `in`. Every write
  is then held to them; documents already stored are not checked.
  """
  def set_field_rules(client, collection, rules) do
    Client.send_command(client, "set_field_rules", %{database: Client.get_database(client), collection: collection, rules: rules})
  end

  # --- Counter fields ---

  @doc """
  The counter fields of `collection`, each with its state (`"next"` is the
  next number).
  """
  def list_counters(client, collection) do
    Client.send_command(client, "list_counters", %{database: Client.get_database(client), collection: collection})
  end

  @doc """
  Declare that the database numbers `field` on insert: each insert gets the
  next number, never twice the same, with no gap. `options` is a map that
  may hold `start`, `reset` ("day", "week", "month", "year"), `timezone`,
  `prefix`, `suffix`, `padding`, `durable` and `node`. An insert then
  leaves the field out and reads it from the answer.
  """
  def create_counter(client, collection, field, options \\ %{}) do
    counter = options |> Map.new() |> Map.put(:field, field)
    Client.send_command(client, "create_counter", %{database: Client.get_database(client), collection: collection, counter: counter})
  end

  def delete_counter(client, collection, field) do
    Client.send_command(client, "delete_counter", %{database: Client.get_database(client), collection: collection, field: field})
  end

  @doc """
  Make `next` the number the next insert gets. Refused when a document
  already holds it.
  """
  def set_counter_next(client, collection, field, next) do
    Client.send_command(client, "set_counter_next", %{database: Client.get_database(client), collection: collection, field: field, next: next})
  end

  @doc """
  Hand the numbering of `field` to `node` (in a cluster, one node numbers),
  asked of the node that numbers now. With `force: true`, take it for the
  node this client is connected to, for when the node that numbers is lost.
  """
  def move_counter(client, collection, field, opts \\ []) do
    args =
      %{database: Client.get_database(client), collection: collection, field: field, force: opts[:force] == true}
      |> put_opt(:node, opts[:node])

    Client.send_command(client, "move_counter", args)
  end

  defp put_opt(args, _key, nil), do: args
  defp put_opt(args, key, value), do: Map.put(args, key, value)
end
