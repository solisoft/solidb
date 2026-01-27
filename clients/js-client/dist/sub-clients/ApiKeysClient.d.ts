import type { Client } from '../Client';
export declare class ApiKeysClient {
    private client;
    constructor(client: Client);
    list(): Promise<any[]>;
    create(name: string, permissions: Array<{
        action: string;
        scope: string;
        database?: string;
    }>, expiresAt?: string): Promise<any>;
    get(keyId: string): Promise<any>;
    delete(keyId: string): Promise<void>;
    regenerate(keyId: string): Promise<any>;
}
