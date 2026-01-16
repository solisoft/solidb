import type { Client } from '../Client';

export class VectorClient {
    constructor(private client: Client) {}

    async createIndex(
        collection: string,
        name: string,
        field: string,
        dimensions: number,
        options?: { metric?: string; [key: string]: any }
    ): Promise<any> {
        return this.client.sendCommand('create_vector_index', {
            database: this.client.database,
            collection,
            name,
            field,
            dimensions,
            ...options
        });
    }

    async listIndexes(collection: string): Promise<any[]> {
        return (await this.client.sendCommand('list_vector_indexes', {
            database: this.client.database,
            collection
        })) || [];
    }

    async deleteIndex(collection: string, indexName: string): Promise<void> {
        await this.client.sendCommand('delete_vector_index', {
            database: this.client.database,
            collection,
            index_name: indexName
        });
    }

    async search(
        collection: string,
        vector: number[],
        limit: number,
        filter?: Record<string, any>
    ): Promise<any[]> {
        return (await this.client.sendCommand('vector_search', {
            database: this.client.database,
            collection,
            vector,
            limit,
            filter
        })) || [];
    }

    async searchByDocument(
        collection: string,
        docKey: string,
        field: string,
        limit: number,
        filter?: Record<string, any>
    ): Promise<any[]> {
        return (await this.client.sendCommand('vector_search_by_doc', {
            database: this.client.database,
            collection,
            doc_key: docKey,
            field,
            limit,
            filter
        })) || [];
    }

    async quantize(collection: string, indexName: string, quantization: string): Promise<void> {
        await this.client.sendCommand('vector_quantize', {
            database: this.client.database,
            collection,
            index_name: indexName,
            quantization
        });
    }

    async dequantize(collection: string, indexName: string): Promise<void> {
        await this.client.sendCommand('vector_dequantize', {
            database: this.client.database,
            collection,
            index_name: indexName
        });
    }

    async getIndexInfo(collection: string, indexName: string): Promise<any> {
        return this.client.sendCommand('vector_index_info', {
            database: this.client.database,
            collection,
            index_name: indexName
        });
    }
}
