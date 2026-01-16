defmodule SoliDB.Columnar do
  @moduledoc """
  Columnar storage management API for SoliDB.
  """

  alias SoliDB.Client

  def create(client, name, columns) do
    Client.send_command(client, "create_columnar_table", %{database: Client.get_database(client), name: name, columns: columns})
  end

  def list(client) do
    Client.send_command(client, "list_columnar_tables", %{database: Client.get_database(client)})
  end

  def get(client, name) do
    Client.send_command(client, "get_columnar_table", %{database: Client.get_database(client), name: name})
  end

  def delete(client, name) do
    Client.send_command(client, "delete_columnar_table", %{database: Client.get_database(client), name: name})
  end

  def insert(client, name, rows) do
    Client.send_command(client, "columnar_insert", %{database: Client.get_database(client), name: name, rows: rows})
  end

  def query(client, name, query, opts \\ []) do
    args = %{database: Client.get_database(client), name: name, query: query}
    args = if opts[:params], do: Map.put(args, :params, opts[:params]), else: args
    Client.send_command(client, "columnar_query", args)
  end

  def aggregate(client, name, aggregation) do
    Client.send_command(client, "columnar_aggregate", %{database: Client.get_database(client), name: name, aggregation: aggregation})
  end

  def create_index(client, table_name, index_name, column, opts \\ []) do
    args = %{database: Client.get_database(client), table_name: table_name, index_name: index_name, column: column}
    args = if opts[:index_type], do: Map.put(args, :index_type, opts[:index_type]), else: args
    Client.send_command(client, "columnar_create_index", args)
  end

  def list_indexes(client, table_name) do
    Client.send_command(client, "columnar_list_indexes", %{database: Client.get_database(client), table_name: table_name})
  end

  def delete_index(client, table_name, index_name) do
    Client.send_command(client, "columnar_delete_index", %{database: Client.get_database(client), table_name: table_name, index_name: index_name})
  end

  def add_column(client, table_name, column_name, column_type, opts \\ []) do
    args = %{database: Client.get_database(client), table_name: table_name, column_name: column_name, column_type: column_type}
    args = if opts[:default_value], do: Map.put(args, :default_value, opts[:default_value]), else: args
    Client.send_command(client, "columnar_add_column", args)
  end

  def drop_column(client, table_name, column_name) do
    Client.send_command(client, "columnar_drop_column", %{database: Client.get_database(client), table_name: table_name, column_name: column_name})
  end

  def stats(client, table_name) do
    Client.send_command(client, "columnar_stats", %{database: Client.get_database(client), table_name: table_name})
  end
end
