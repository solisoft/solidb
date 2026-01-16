module SoliDB
  class CronClient
    def initialize(client)
      @client = client
    end

    def list
      @client.send_command("list_cron_jobs", database: @client.database) || []
    end

    def create(name:, schedule:, script_path:, params: nil, enabled: nil, description: nil)
      args = {
        database: @client.database,
        name: name,
        schedule: schedule,
        script_path: script_path
      }
      args[:params] = params if params
      args[:enabled] = enabled unless enabled.nil?
      args[:description] = description if description
      @client.send_command("create_cron_job", args)
    end

    def get(cron_id)
      @client.send_command("get_cron_job", database: @client.database, cron_id: cron_id)
    end

    def update(cron_id, updates)
      @client.send_command("update_cron_job", database: @client.database, cron_id: cron_id, updates: updates)
    end

    def delete(cron_id)
      @client.send_command("delete_cron_job", database: @client.database, cron_id: cron_id)
      nil
    end

    def toggle(cron_id, enabled)
      @client.send_command("toggle_cron_job", database: @client.database, cron_id: cron_id, enabled: enabled)
      nil
    end
  end
end
