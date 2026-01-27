"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.UsersClient = void 0;
class UsersClient {
    constructor(client) {
        this.client = client;
    }
    async list() {
        return (await this.client.sendCommand('list_users', {})) || [];
    }
    async create(username, password, roles) {
        return this.client.sendCommand('create_user', {
            username,
            password,
            roles
        });
    }
    async get(username) {
        return this.client.sendCommand('get_user', {
            username
        });
    }
    async delete(username) {
        await this.client.sendCommand('delete_user', {
            username
        });
    }
    async getRoles(username) {
        return (await this.client.sendCommand('get_user_roles', {
            username
        })) || [];
    }
    async assignRole(username, role, database) {
        await this.client.sendCommand('assign_role', {
            username,
            role,
            database
        });
    }
    async revokeRole(username, role, database) {
        await this.client.sendCommand('revoke_role', {
            username,
            role,
            database
        });
    }
    async me() {
        return this.client.sendCommand('get_current_user', {});
    }
    async myPermissions() {
        return (await this.client.sendCommand('get_my_permissions', {})) || [];
    }
    async changePassword(username, oldPassword, newPassword) {
        await this.client.sendCommand('change_password', {
            username,
            old_password: oldPassword,
            new_password: newPassword
        });
    }
}
exports.UsersClient = UsersClient;
