module SoliDB
  class CollectionsOpsClient
    def initialize(client)
      @client = client
    end

    def truncate(collection)
      @client.send_command("truncate_collection", database: @client.database, collection: collection)
      nil
    end

    def compact(collection)
      @client.send_command("compact_collection", database: @client.database, collection: collection)
      nil
    end

    def stats(collection)
      @client.send_command("collection_stats", database: @client.database, collection: collection)
    end

    def prune(collection, older_than: nil, field: nil)
      args = { database: @client.database, collection: collection }
      args[:older_than] = older_than if older_than
      args[:field] = field if field
      @client.send_command("prune_collection", args)
    end

    def recount(collection)
      @client.send_command("recount_collection", database: @client.database, collection: collection)
    end

    def repair(collection)
      @client.send_command("repair_collection", database: @client.database, collection: collection)
    end

    def set_schema(collection, schema)
      @client.send_command("set_collection_schema", database: @client.database, collection: collection, schema: schema)
      nil
    end

    def get_schema(collection)
      @client.send_command("get_collection_schema", database: @client.database, collection: collection)
    end

    def delete_schema(collection)
      @client.send_command("delete_collection_schema", database: @client.database, collection: collection)
      nil
    end

    def export(collection, format)
      @client.send_command("export_collection", database: @client.database, collection: collection, format: format)
    end

    def import(collection, data, format)
      @client.send_command("import_collection", database: @client.database, collection: collection, data: data, format: format)
    end

    def get_sharding(collection)
      @client.send_command("get_collection_sharding", database: @client.database, collection: collection)
    end

    def set_sharding(collection, config)
      args = { database: @client.database, collection: collection }.merge(config)
      @client.send_command("set_collection_sharding", args)
      nil
    end
  end
end
