import type { Client } from '../Client';
export declare class EnvClient {
    private client;
    constructor(client: Client);
    list(): Promise<Record<string, any>>;
    get(key: string): Promise<any>;
    set(key: string, value: any): Promise<void>;
    delete(key: string): Promise<void>;
    setBulk(vars: Record<string, any>): Promise<void>;
}
