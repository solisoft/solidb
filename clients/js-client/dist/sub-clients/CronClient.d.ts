import type { Client } from '../Client';
export declare class CronClient {
    private client;
    constructor(client: Client);
    list(): Promise<any[]>;
    create(name: string, schedule: string, scriptPath: string, options?: {
        params?: Record<string, any>;
        enabled?: boolean;
        description?: string;
    }): Promise<any>;
    get(cronId: string): Promise<any>;
    update(cronId: string, updates: Record<string, any>): Promise<any>;
    delete(cronId: string): Promise<void>;
    toggle(cronId: string, enabled: boolean): Promise<void>;
}
