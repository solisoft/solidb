import type { Client } from '../Client';
export declare class IndexesClient {
    private client;
    constructor(client: Client);
    rebuild(collection: string, indexName: string): Promise<void>;
    rebuildAll(collection: string): Promise<void>;
    hybridSearch(collection: string, query: Record<string, any>): Promise<any[]>;
    analyze(collection: string, indexName: string): Promise<any>;
    getUsageStats(collection: string): Promise<any>;
}
