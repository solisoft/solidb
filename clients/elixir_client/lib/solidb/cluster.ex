defmodule SoliDB.Cluster do
  @moduledoc """
  Cluster management API for SoliDB.
  """

  alias SoliDB.Client

  def status(client) do
    Client.send_command(client, "cluster_status", %{})
  end

  def info(client) do
    Client.send_command(client, "cluster_info", %{})
  end

  def remove_node(client, node_id) do
    Client.send_command(client, "cluster_remove_node", %{node_id: node_id})
  end

  def rebalance(client) do
    Client.send_command(client, "cluster_rebalance", %{})
  end

  def cleanup(client) do
    Client.send_command(client, "cluster_cleanup", %{})
  end

  def reshard(client, num_shards) do
    Client.send_command(client, "cluster_reshard", %{num_shards: num_shards})
  end

  def get_nodes(client) do
    Client.send_command(client, "cluster_get_nodes", %{})
  end

  def get_shards(client) do
    Client.send_command(client, "cluster_get_shards", %{})
  end
end
