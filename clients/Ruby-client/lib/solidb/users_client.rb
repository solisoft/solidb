module SoliDB
  class UsersClient
    def initialize(client)
      @client = client
    end

    def list
      @client.send_command("list_users", {}) || []
    end

    def create(username:, password:, roles: nil)
      args = { username: username, password: password }
      args[:roles] = roles if roles
      @client.send_command("create_user", args)
    end

    def get(username)
      @client.send_command("get_user", username: username)
    end

    def delete(username)
      @client.send_command("delete_user", username: username)
      nil
    end

    def get_roles(username)
      @client.send_command("get_user_roles", username: username) || []
    end

    def assign_role(username, role, database: nil)
      args = { username: username, role: role }
      args[:database] = database if database
      @client.send_command("assign_role", args)
      nil
    end

    def revoke_role(username, role, database: nil)
      args = { username: username, role: role }
      args[:database] = database if database
      @client.send_command("revoke_role", args)
      nil
    end

    def me
      @client.send_command("get_current_user", {})
    end

    def my_permissions
      @client.send_command("get_my_permissions", {}) || []
    end

    def change_password(username, old_password, new_password)
      @client.send_command("change_password", username: username, old_password: old_password, new_password: new_password)
      nil
    end
  end
end
