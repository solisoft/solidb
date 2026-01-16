module SoliDB
  class TriggersClient
    def initialize(client)
      @client = client
    end

    def list
      @client.send_command("list_triggers", database: @client.database) || []
    end

    def list_by_collection(collection)
      @client.send_command("list_triggers_by_collection", database: @client.database, collection: collection) || []
    end

    def create(name:, collection:, event:, timing:, script_path:, enabled: nil)
      args = {
        database: @client.database,
        name: name,
        collection: collection,
        event: event,
        timing: timing,
        script_path: script_path
      }
      args[:enabled] = enabled unless enabled.nil?
      @client.send_command("create_trigger", args)
    end

    def get(trigger_id)
      @client.send_command("get_trigger", database: @client.database, trigger_id: trigger_id)
    end

    def update(trigger_id, updates)
      @client.send_command("update_trigger", database: @client.database, trigger_id: trigger_id, updates: updates)
    end

    def delete(trigger_id)
      @client.send_command("delete_trigger", database: @client.database, trigger_id: trigger_id)
      nil
    end

    def toggle(trigger_id, enabled)
      @client.send_command("toggle_trigger", database: @client.database, trigger_id: trigger_id, enabled: enabled)
      nil
    end
  end
end
