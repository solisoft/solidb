import type { Client } from '../Client';

export class IndexesClient {
    constructor(private client: Client) {}

    async rebuild(collection: string, indexName: string): Promise<void> {
        await this.client.sendCommand('rebuild_index', {
            database: this.client.database,
            collection,
            index_name: indexName
        });
    }

    async rebuildAll(collection: string): Promise<void> {
        await this.client.sendCommand('rebuild_all_indexes', {
            database: this.client.database,
            collection
        });
    }

    async hybridSearch(collection: string, query: Record<string, any>): Promise<any[]> {
        return (await this.client.sendCommand('hybrid_search', {
            database: this.client.database,
            collection,
            ...query
        })) || [];
    }

    async analyze(collection: string, indexName: string): Promise<any> {
        return this.client.sendCommand('analyze_index', {
            database: this.client.database,
            collection,
            index_name: indexName
        });
    }

    async getUsageStats(collection: string): Promise<any> {
        return this.client.sendCommand('index_usage_stats', {
            database: this.client.database,
            collection
        });
    }
}
