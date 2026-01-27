import type { Client } from '../Client';
export declare class TriggersClient {
    private client;
    constructor(client: Client);
    list(): Promise<any[]>;
    listByCollection(collection: string): Promise<any[]>;
    create(name: string, collection: string, event: string, timing: string, scriptPath: string, enabled?: boolean): Promise<any>;
    get(triggerId: string): Promise<any>;
    update(triggerId: string, updates: Record<string, any>): Promise<any>;
    delete(triggerId: string): Promise<void>;
    toggle(triggerId: string, enabled: boolean): Promise<void>;
}
