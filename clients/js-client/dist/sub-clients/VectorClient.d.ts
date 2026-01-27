import type { Client } from '../Client';
export declare class VectorClient {
    private client;
    constructor(client: Client);
    createIndex(collection: string, name: string, field: string, dimensions: number, options?: {
        metric?: string;
        [key: string]: any;
    }): Promise<any>;
    listIndexes(collection: string): Promise<any[]>;
    deleteIndex(collection: string, indexName: string): Promise<void>;
    search(collection: string, vector: number[], limit: number, filter?: Record<string, any>): Promise<any[]>;
    searchByDocument(collection: string, docKey: string, field: string, limit: number, filter?: Record<string, any>): Promise<any[]>;
    quantize(collection: string, indexName: string, quantization: string): Promise<void>;
    dequantize(collection: string, indexName: string): Promise<void>;
    getIndexInfo(collection: string, indexName: string): Promise<any>;
}
