"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.TTLClient = void 0;
class TTLClient {
    constructor(client) {
        this.client = client;
    }
    async createIndex(collection, name, field, expireAfterSeconds) {
        return this.client.sendCommand('create_ttl_index', {
            database: this.client.database,
            collection,
            name,
            field,
            expire_after_seconds: expireAfterSeconds
        });
    }
    async listIndexes(collection) {
        return (await this.client.sendCommand('list_ttl_indexes', {
            database: this.client.database,
            collection
        })) || [];
    }
    async deleteIndex(collection, indexName) {
        await this.client.sendCommand('delete_ttl_index', {
            database: this.client.database,
            collection,
            index_name: indexName
        });
    }
    async updateExpiration(collection, indexName, expireAfterSeconds) {
        await this.client.sendCommand('update_ttl_expiration', {
            database: this.client.database,
            collection,
            index_name: indexName,
            expire_after_seconds: expireAfterSeconds
        });
    }
    async getIndexInfo(collection, indexName) {
        return this.client.sendCommand('ttl_index_info', {
            database: this.client.database,
            collection,
            index_name: indexName
        });
    }
    async runCleanup(collection) {
        return this.client.sendCommand('ttl_run_cleanup', {
            database: this.client.database,
            collection
        });
    }
}
exports.TTLClient = TTLClient;
