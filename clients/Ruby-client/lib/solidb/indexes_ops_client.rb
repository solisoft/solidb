module SoliDB
  class IndexesOpsClient
    def initialize(client)
      @client = client
    end

    def rebuild(collection, index_name)
      @client.send_command("rebuild_index", database: @client.database, collection: collection, index_name: index_name)
      nil
    end

    def rebuild_all(collection)
      @client.send_command("rebuild_all_indexes", database: @client.database, collection: collection)
      nil
    end

    def hybrid_search(collection, query)
      args = { database: @client.database, collection: collection }.merge(query)
      @client.send_command("hybrid_search", args) || []
    end

    def analyze(collection, index_name)
      @client.send_command("analyze_index", database: @client.database, collection: collection, index_name: index_name)
    end

    def get_usage_stats(collection)
      @client.send_command("index_usage_stats", database: @client.database, collection: collection)
    end
  end
end
