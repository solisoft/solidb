import type { Client } from '../Client';
export declare class TTLClient {
    private client;
    constructor(client: Client);
    createIndex(collection: string, name: string, field: string, expireAfterSeconds: number): Promise<any>;
    listIndexes(collection: string): Promise<any[]>;
    deleteIndex(collection: string, indexName: string): Promise<void>;
    updateExpiration(collection: string, indexName: string, expireAfterSeconds: number): Promise<void>;
    getIndexInfo(collection: string, indexName: string): Promise<any>;
    runCleanup(collection: string): Promise<any>;
}
