"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.TriggersClient = void 0;
class TriggersClient {
    constructor(client) {
        this.client = client;
    }
    async list() {
        return (await this.client.sendCommand('list_triggers', {
            database: this.client.database
        })) || [];
    }
    async listByCollection(collection) {
        return (await this.client.sendCommand('list_triggers_by_collection', {
            database: this.client.database,
            collection
        })) || [];
    }
    async create(name, collection, event, timing, scriptPath, enabled) {
        return this.client.sendCommand('create_trigger', {
            database: this.client.database,
            name,
            collection,
            event,
            timing,
            script_path: scriptPath,
            enabled
        });
    }
    async get(triggerId) {
        return this.client.sendCommand('get_trigger', {
            database: this.client.database,
            trigger_id: triggerId
        });
    }
    async update(triggerId, updates) {
        return this.client.sendCommand('update_trigger', {
            database: this.client.database,
            trigger_id: triggerId,
            updates
        });
    }
    async delete(triggerId) {
        await this.client.sendCommand('delete_trigger', {
            database: this.client.database,
            trigger_id: triggerId
        });
    }
    async toggle(triggerId, enabled) {
        await this.client.sendCommand('toggle_trigger', {
            database: this.client.database,
            trigger_id: triggerId,
            enabled
        });
    }
}
exports.TriggersClient = TriggersClient;
