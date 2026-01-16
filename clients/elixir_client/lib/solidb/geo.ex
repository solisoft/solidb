defmodule SoliDB.Geo do
  @moduledoc """
  Geo-spatial index and query API for SoliDB.
  """

  alias SoliDB.Client

  def create_index(client, collection, name, fields, opts \\ []) do
    args = %{database: Client.get_database(client), collection: collection, name: name, fields: fields}
    args = if Keyword.has_key?(opts, :geo_json), do: Map.put(args, :geo_json, opts[:geo_json]), else: args
    Client.send_command(client, "create_geo_index", args)
  end

  def list_indexes(client, collection) do
    Client.send_command(client, "list_geo_indexes", %{database: Client.get_database(client), collection: collection})
  end

  def delete_index(client, collection, index_name) do
    Client.send_command(client, "delete_geo_index", %{database: Client.get_database(client), collection: collection, index_name: index_name})
  end

  def near(client, collection, latitude, longitude, radius, opts \\ []) do
    args = %{database: Client.get_database(client), collection: collection, latitude: latitude, longitude: longitude, radius: radius}
    args = if opts[:limit], do: Map.put(args, :limit, opts[:limit]), else: args
    Client.send_command(client, "geo_near", args)
  end

  def within(client, collection, geometry) do
    Client.send_command(client, "geo_within", %{database: Client.get_database(client), collection: collection, geometry: geometry})
  end

  def distance(client, lat1, lon1, lat2, lon2) do
    Client.send_command(client, "geo_distance", %{lat1: lat1, lon1: lon1, lat2: lat2, lon2: lon2})
  end

  def intersects(client, collection, geometry) do
    Client.send_command(client, "geo_intersects", %{database: Client.get_database(client), collection: collection, geometry: geometry})
  end
end
