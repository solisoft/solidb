"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.IndexesClient = void 0;
class IndexesClient {
    constructor(client) {
        this.client = client;
    }
    async rebuild(collection, indexName) {
        await this.client.sendCommand('rebuild_index', {
            database: this.client.database,
            collection,
            index_name: indexName
        });
    }
    async rebuildAll(collection) {
        await this.client.sendCommand('rebuild_all_indexes', {
            database: this.client.database,
            collection
        });
    }
    async hybridSearch(collection, query) {
        return (await this.client.sendCommand('hybrid_search', {
            database: this.client.database,
            collection,
            ...query
        })) || [];
    }
    async analyze(collection, indexName) {
        return this.client.sendCommand('analyze_index', {
            database: this.client.database,
            collection,
            index_name: indexName
        });
    }
    async getUsageStats(collection) {
        return this.client.sendCommand('index_usage_stats', {
            database: this.client.database,
            collection
        });
    }
}
exports.IndexesClient = IndexesClient;
