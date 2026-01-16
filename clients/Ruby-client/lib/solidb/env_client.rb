module SoliDB
  class EnvClient
    def initialize(client)
      @client = client
    end

    def list
      @client.send_command("list_env_vars", database: @client.database) || {}
    end

    def get(key)
      @client.send_command("get_env_var", database: @client.database, key: key)
    end

    def set(key, value)
      @client.send_command("set_env_var", database: @client.database, key: key, value: value)
      nil
    end

    def delete(key)
      @client.send_command("delete_env_var", database: @client.database, key: key)
      nil
    end

    def set_bulk(vars)
      @client.send_command("set_env_vars_bulk", database: @client.database, vars: vars)
      nil
    end
  end
end
