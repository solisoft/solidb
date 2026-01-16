module SoliDB
  class ApiKeysClient
    def initialize(client)
      @client = client
    end

    def list
      @client.send_command("list_api_keys", {}) || []
    end

    def create(name:, permissions:, expires_at: nil)
      args = { name: name, permissions: permissions }
      args[:expires_at] = expires_at if expires_at
      @client.send_command("create_api_key", args)
    end

    def get(key_id)
      @client.send_command("get_api_key", key_id: key_id)
    end

    def delete(key_id)
      @client.send_command("delete_api_key", key_id: key_id)
      nil
    end

    def regenerate(key_id)
      @client.send_command("regenerate_api_key", key_id: key_id)
    end
  end
end
