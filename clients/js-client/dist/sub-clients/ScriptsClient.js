"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.ScriptsClient = void 0;
class ScriptsClient {
    constructor(client) {
        this.client = client;
    }
    async create(name, path, methods, code, options) {
        return this.client.sendCommand('create_script', {
            database: this.client.database,
            name,
            path,
            methods,
            code,
            ...options
        });
    }
    async list() {
        return (await this.client.sendCommand('list_scripts', {
            database: this.client.database
        })) || [];
    }
    async get(scriptId) {
        return this.client.sendCommand('get_script', {
            database: this.client.database,
            script_id: scriptId
        });
    }
    async update(scriptId, updates) {
        return this.client.sendCommand('update_script', {
            database: this.client.database,
            script_id: scriptId,
            updates
        });
    }
    async delete(scriptId) {
        await this.client.sendCommand('delete_script', {
            database: this.client.database,
            script_id: scriptId
        });
    }
    async getStats() {
        return this.client.sendCommand('get_script_stats', {});
    }
}
exports.ScriptsClient = ScriptsClient;
