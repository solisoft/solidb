module SoliDB
  class TTLClient
    def initialize(client)
      @client = client
    end

    def create_index(collection, name:, field:, expire_after_seconds:)
      @client.send_command("create_ttl_index",
        database: @client.database,
        collection: collection,
        name: name,
        field: field,
        expire_after_seconds: expire_after_seconds
      )
    end

    def list_indexes(collection)
      @client.send_command("list_ttl_indexes", database: @client.database, collection: collection) || []
    end

    def delete_index(collection, index_name)
      @client.send_command("delete_ttl_index", database: @client.database, collection: collection, index_name: index_name)
      nil
    end

    def update_expiration(collection, index_name, expire_after_seconds)
      @client.send_command("update_ttl_expiration",
        database: @client.database,
        collection: collection,
        index_name: index_name,
        expire_after_seconds: expire_after_seconds
      )
      nil
    end

    def get_index_info(collection, index_name)
      @client.send_command("ttl_index_info", database: @client.database, collection: collection, index_name: index_name)
    end

    def run_cleanup(collection)
      @client.send_command("ttl_run_cleanup", database: @client.database, collection: collection)
    end
  end
end
