module SoliDB
  class RolesClient
    def initialize(client)
      @client = client
    end

    def list
      @client.send_command("list_roles", {}) || []
    end

    def create(name:, permissions:, description: nil)
      args = { name: name, permissions: permissions }
      args[:description] = description if description
      @client.send_command("create_role", args)
    end

    def get(name)
      @client.send_command("get_role", role_name: name)
    end

    def update(name, permissions:, description: nil)
      args = { role_name: name, permissions: permissions }
      args[:description] = description if description
      @client.send_command("update_role", args)
    end

    def delete(name)
      @client.send_command("delete_role", role_name: name)
      nil
    end
  end
end
