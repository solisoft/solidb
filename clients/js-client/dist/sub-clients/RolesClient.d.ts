import type { Client } from '../Client';
export interface Permission {
    action: string;
    scope: string;
    database?: string;
}
export declare class RolesClient {
    private client;
    constructor(client: Client);
    list(): Promise<any[]>;
    create(name: string, permissions: Permission[], description?: string): Promise<any>;
    get(name: string): Promise<any>;
    update(name: string, permissions: Permission[], description?: string): Promise<any>;
    delete(name: string): Promise<void>;
}
