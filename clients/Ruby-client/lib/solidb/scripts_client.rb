module SoliDB
  class ScriptsClient
    def initialize(client)
      @client = client
    end

    def create(name:, path:, methods:, code:, description: nil, collection: nil)
      params = {
        database: @client.database,
        name: name,
        path: path,
        methods: methods,
        code: code
      }
      params[:description] = description if description
      params[:collection] = collection if collection
      @client.send_command("create_script", params)
    end

    def list
      @client.send_command("list_scripts", database: @client.database) || []
    end

    def get(script_id)
      @client.send_command("get_script", database: @client.database, script_id: script_id)
    end

    def update(script_id, updates)
      @client.send_command("update_script", database: @client.database, script_id: script_id, updates: updates)
    end

    def delete(script_id)
      @client.send_command("delete_script", database: @client.database, script_id: script_id)
      nil
    end

    def get_stats
      @client.send_command("get_script_stats", {})
    end
  end
end
