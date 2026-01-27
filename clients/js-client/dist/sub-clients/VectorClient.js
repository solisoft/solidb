"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.VectorClient = void 0;
class VectorClient {
    constructor(client) {
        this.client = client;
    }
    async createIndex(collection, name, field, dimensions, options) {
        return this.client.sendCommand('create_vector_index', {
            database: this.client.database,
            collection,
            name,
            field,
            dimensions,
            ...options
        });
    }
    async listIndexes(collection) {
        return (await this.client.sendCommand('list_vector_indexes', {
            database: this.client.database,
            collection
        })) || [];
    }
    async deleteIndex(collection, indexName) {
        await this.client.sendCommand('delete_vector_index', {
            database: this.client.database,
            collection,
            index_name: indexName
        });
    }
    async search(collection, vector, limit, filter) {
        return (await this.client.sendCommand('vector_search', {
            database: this.client.database,
            collection,
            vector,
            limit,
            filter
        })) || [];
    }
    async searchByDocument(collection, docKey, field, limit, filter) {
        return (await this.client.sendCommand('vector_search_by_doc', {
            database: this.client.database,
            collection,
            doc_key: docKey,
            field,
            limit,
            filter
        })) || [];
    }
    async quantize(collection, indexName, quantization) {
        await this.client.sendCommand('vector_quantize', {
            database: this.client.database,
            collection,
            index_name: indexName,
            quantization
        });
    }
    async dequantize(collection, indexName) {
        await this.client.sendCommand('vector_dequantize', {
            database: this.client.database,
            collection,
            index_name: indexName
        });
    }
    async getIndexInfo(collection, indexName) {
        return this.client.sendCommand('vector_index_info', {
            database: this.client.database,
            collection,
            index_name: indexName
        });
    }
}
exports.VectorClient = VectorClient;
