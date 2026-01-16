module SoliDB
  class GeoClient
    def initialize(client)
      @client = client
    end

    def create_index(collection, name:, fields:, geo_json: nil)
      args = { database: @client.database, collection: collection, name: name, fields: fields }
      args[:geo_json] = geo_json unless geo_json.nil?
      @client.send_command("create_geo_index", args)
    end

    def list_indexes(collection)
      @client.send_command("list_geo_indexes", database: @client.database, collection: collection) || []
    end

    def delete_index(collection, index_name)
      @client.send_command("delete_geo_index", database: @client.database, collection: collection, index_name: index_name)
      nil
    end

    def near(collection, latitude:, longitude:, radius:, limit: nil)
      args = {
        database: @client.database,
        collection: collection,
        latitude: latitude,
        longitude: longitude,
        radius: radius
      }
      args[:limit] = limit if limit
      @client.send_command("geo_near", args) || []
    end

    def within(collection, geometry:)
      @client.send_command("geo_within", database: @client.database, collection: collection, geometry: geometry) || []
    end

    def distance(lat1:, lon1:, lat2:, lon2:)
      @client.send_command("geo_distance", lat1: lat1, lon1: lon1, lat2: lat2, lon2: lon2)
    end

    def intersects(collection, geometry:)
      @client.send_command("geo_intersects", database: @client.database, collection: collection, geometry: geometry) || []
    end
  end
end
