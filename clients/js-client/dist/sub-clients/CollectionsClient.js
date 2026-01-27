"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.CollectionsClient = void 0;
class CollectionsClient {
    constructor(client) {
        this.client = client;
    }
    async truncate(collection) {
        await this.client.sendCommand('truncate_collection', {
            database: this.client.database,
            collection
        });
    }
    async compact(collection) {
        await this.client.sendCommand('compact_collection', {
            database: this.client.database,
            collection
        });
    }
    async stats(collection) {
        return this.client.sendCommand('collection_stats', {
            database: this.client.database,
            collection
        });
    }
    async prune(collection, options) {
        return this.client.sendCommand('prune_collection', {
            database: this.client.database,
            collection,
            older_than: options?.olderThan,
            field: options?.field
        });
    }
    async recount(collection) {
        return this.client.sendCommand('recount_collection', {
            database: this.client.database,
            collection
        });
    }
    async repair(collection) {
        return this.client.sendCommand('repair_collection', {
            database: this.client.database,
            collection
        });
    }
    async setSchema(collection, schema) {
        await this.client.sendCommand('set_collection_schema', {
            database: this.client.database,
            collection,
            schema
        });
    }
    async getSchema(collection) {
        return this.client.sendCommand('get_collection_schema', {
            database: this.client.database,
            collection
        });
    }
    async deleteSchema(collection) {
        await this.client.sendCommand('delete_collection_schema', {
            database: this.client.database,
            collection
        });
    }
    async export(collection, format) {
        return this.client.sendCommand('export_collection', {
            database: this.client.database,
            collection,
            format
        });
    }
    async import(collection, data, format) {
        return this.client.sendCommand('import_collection', {
            database: this.client.database,
            collection,
            data,
            format
        });
    }
    async getSharding(collection) {
        return this.client.sendCommand('get_collection_sharding', {
            database: this.client.database,
            collection
        });
    }
    async setSharding(collection, config) {
        await this.client.sendCommand('set_collection_sharding', {
            database: this.client.database,
            collection,
            ...config
        });
    }
}
exports.CollectionsClient = CollectionsClient;
