module SoliDB
  class ClusterClient
    def initialize(client)
      @client = client
    end

    def status
      @client.send_command("cluster_status", {})
    end

    def info
      @client.send_command("cluster_info", {})
    end

    def remove_node(node_id)
      @client.send_command("cluster_remove_node", node_id: node_id)
      nil
    end

    def rebalance
      @client.send_command("cluster_rebalance", {})
      nil
    end

    def cleanup
      @client.send_command("cluster_cleanup", {})
      nil
    end

    def reshard(num_shards)
      @client.send_command("cluster_reshard", num_shards: num_shards)
      nil
    end

    def get_nodes
      @client.send_command("cluster_get_nodes", {}) || []
    end

    def get_shards
      @client.send_command("cluster_get_shards", {}) || []
    end
  end
end
