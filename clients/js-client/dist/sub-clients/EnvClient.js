"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.EnvClient = void 0;
class EnvClient {
    constructor(client) {
        this.client = client;
    }
    async list() {
        return (await this.client.sendCommand('list_env_vars', {
            database: this.client.database
        })) || {};
    }
    async get(key) {
        return this.client.sendCommand('get_env_var', {
            database: this.client.database,
            key
        });
    }
    async set(key, value) {
        await this.client.sendCommand('set_env_var', {
            database: this.client.database,
            key,
            value
        });
    }
    async delete(key) {
        await this.client.sendCommand('delete_env_var', {
            database: this.client.database,
            key
        });
    }
    async setBulk(vars) {
        await this.client.sendCommand('set_env_vars_bulk', {
            database: this.client.database,
            vars
        });
    }
}
exports.EnvClient = EnvClient;
