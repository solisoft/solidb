import type { Client } from '../Client';

export class TTLClient {
    constructor(private client: Client) {}

    async createIndex(
        collection: string,
        name: string,
        field: string,
        expireAfterSeconds: number
    ): Promise<any> {
        return this.client.sendCommand('create_ttl_index', {
            database: this.client.database,
            collection,
            name,
            field,
            expire_after_seconds: expireAfterSeconds
        });
    }

    async listIndexes(collection: string): Promise<any[]> {
        return (await this.client.sendCommand('list_ttl_indexes', {
            database: this.client.database,
            collection
        })) || [];
    }

    async deleteIndex(collection: string, indexName: string): Promise<void> {
        await this.client.sendCommand('delete_ttl_index', {
            database: this.client.database,
            collection,
            index_name: indexName
        });
    }

    async updateExpiration(collection: string, indexName: string, expireAfterSeconds: number): Promise<void> {
        await this.client.sendCommand('update_ttl_expiration', {
            database: this.client.database,
            collection,
            index_name: indexName,
            expire_after_seconds: expireAfterSeconds
        });
    }

    async getIndexInfo(collection: string, indexName: string): Promise<any> {
        return this.client.sendCommand('ttl_index_info', {
            database: this.client.database,
            collection,
            index_name: indexName
        });
    }

    async runCleanup(collection: string): Promise<any> {
        return this.client.sendCommand('ttl_run_cleanup', {
            database: this.client.database,
            collection
        });
    }
}
