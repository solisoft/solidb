import type { Client } from '../Client';
export declare class CollectionsClient {
    private client;
    constructor(client: Client);
    truncate(collection: string): Promise<void>;
    compact(collection: string): Promise<void>;
    stats(collection: string): Promise<any>;
    prune(collection: string, options?: {
        olderThan?: string;
        field?: string;
    }): Promise<any>;
    recount(collection: string): Promise<any>;
    repair(collection: string): Promise<any>;
    setSchema(collection: string, schema: Record<string, any>): Promise<void>;
    getSchema(collection: string): Promise<any>;
    deleteSchema(collection: string): Promise<void>;
    export(collection: string, format: string): Promise<any>;
    import(collection: string, data: any, format: string): Promise<any>;
    getSharding(collection: string): Promise<any>;
    setSharding(collection: string, config: Record<string, any>): Promise<void>;
}
