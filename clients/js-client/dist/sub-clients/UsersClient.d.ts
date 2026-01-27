import type { Client } from '../Client';
export declare class UsersClient {
    private client;
    constructor(client: Client);
    list(): Promise<any[]>;
    create(username: string, password: string, roles?: string[]): Promise<any>;
    get(username: string): Promise<any>;
    delete(username: string): Promise<void>;
    getRoles(username: string): Promise<any[]>;
    assignRole(username: string, role: string, database?: string): Promise<void>;
    revokeRole(username: string, role: string, database?: string): Promise<void>;
    me(): Promise<any>;
    myPermissions(): Promise<any[]>;
    changePassword(username: string, oldPassword: string, newPassword: string): Promise<void>;
}
