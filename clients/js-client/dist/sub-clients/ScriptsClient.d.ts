import type { Client } from '../Client';
export declare class ScriptsClient {
    private client;
    constructor(client: Client);
    create(name: string, path: string, methods: string[], code: string, options?: {
        description?: string;
        collection?: string;
    }): Promise<any>;
    list(): Promise<any[]>;
    get(scriptId: string): Promise<any>;
    update(scriptId: string, updates: Record<string, any>): Promise<any>;
    delete(scriptId: string): Promise<void>;
    getStats(): Promise<any>;
}
