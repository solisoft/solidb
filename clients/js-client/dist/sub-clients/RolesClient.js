"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.RolesClient = void 0;
class RolesClient {
    constructor(client) {
        this.client = client;
    }
    async list() {
        return (await this.client.sendCommand('list_roles', {})) || [];
    }
    async create(name, permissions, description) {
        return this.client.sendCommand('create_role', {
            name,
            permissions,
            description
        });
    }
    async get(name) {
        return this.client.sendCommand('get_role', {
            role_name: name
        });
    }
    async update(name, permissions, description) {
        return this.client.sendCommand('update_role', {
            role_name: name,
            permissions,
            description
        });
    }
    async delete(name) {
        await this.client.sendCommand('delete_role', {
            role_name: name
        });
    }
}
exports.RolesClient = RolesClient;
