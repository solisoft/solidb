module SoliDB
  class VectorClient
    def initialize(client)
      @client = client
    end

    def create_index(collection, name:, field:, dimensions:, metric: nil, **options)
      args = {
        database: @client.database,
        collection: collection,
        name: name,
        field: field,
        dimensions: dimensions
      }
      args[:metric] = metric if metric
      args.merge!(options)
      @client.send_command("create_vector_index", args)
    end

    def list_indexes(collection)
      @client.send_command("list_vector_indexes", database: @client.database, collection: collection) || []
    end

    def delete_index(collection, index_name)
      @client.send_command("delete_vector_index", database: @client.database, collection: collection, index_name: index_name)
      nil
    end

    def search(collection, vector:, limit:, filter: nil)
      args = {
        database: @client.database,
        collection: collection,
        vector: vector,
        limit: limit
      }
      args[:filter] = filter if filter
      @client.send_command("vector_search", args) || []
    end

    def search_by_document(collection, doc_key:, field:, limit:, filter: nil)
      args = {
        database: @client.database,
        collection: collection,
        doc_key: doc_key,
        field: field,
        limit: limit
      }
      args[:filter] = filter if filter
      @client.send_command("vector_search_by_doc", args) || []
    end

    def quantize(collection, index_name, quantization)
      @client.send_command("vector_quantize", database: @client.database, collection: collection, index_name: index_name, quantization: quantization)
      nil
    end

    def dequantize(collection, index_name)
      @client.send_command("vector_dequantize", database: @client.database, collection: collection, index_name: index_name)
      nil
    end

    def get_index_info(collection, index_name)
      @client.send_command("vector_index_info", database: @client.database, collection: collection, index_name: index_name)
    end
  end
end
