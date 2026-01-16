import type { Client } from '../Client';

export class CollectionsClient {
    constructor(private client: Client) {}

    async truncate(collection: string): Promise<void> {
        await this.client.sendCommand('truncate_collection', {
            database: this.client.database,
            collection
        });
    }

    async compact(collection: string): Promise<void> {
        await this.client.sendCommand('compact_collection', {
            database: this.client.database,
            collection
        });
    }

    async stats(collection: string): Promise<any> {
        return this.client.sendCommand('collection_stats', {
            database: this.client.database,
            collection
        });
    }

    async prune(collection: string, options?: { olderThan?: string; field?: string }): Promise<any> {
        return this.client.sendCommand('prune_collection', {
            database: this.client.database,
            collection,
            older_than: options?.olderThan,
            field: options?.field
        });
    }

    async recount(collection: string): Promise<any> {
        return this.client.sendCommand('recount_collection', {
            database: this.client.database,
            collection
        });
    }

    async repair(collection: string): Promise<any> {
        return this.client.sendCommand('repair_collection', {
            database: this.client.database,
            collection
        });
    }

    async setSchema(collection: string, schema: Record<string, any>): Promise<void> {
        await this.client.sendCommand('set_collection_schema', {
            database: this.client.database,
            collection,
            schema
        });
    }

    async getSchema(collection: string): Promise<any> {
        return this.client.sendCommand('get_collection_schema', {
            database: this.client.database,
            collection
        });
    }

    async deleteSchema(collection: string): Promise<void> {
        await this.client.sendCommand('delete_collection_schema', {
            database: this.client.database,
            collection
        });
    }

    async export(collection: string, format: string): Promise<any> {
        return this.client.sendCommand('export_collection', {
            database: this.client.database,
            collection,
            format
        });
    }

    async import(collection: string, data: any, format: string): Promise<any> {
        return this.client.sendCommand('import_collection', {
            database: this.client.database,
            collection,
            data,
            format
        });
    }

    async getSharding(collection: string): Promise<any> {
        return this.client.sendCommand('get_collection_sharding', {
            database: this.client.database,
            collection
        });
    }

    async setSharding(collection: string, config: Record<string, any>): Promise<void> {
        await this.client.sendCommand('set_collection_sharding', {
            database: this.client.database,
            collection,
            ...config
        });
    }
}
