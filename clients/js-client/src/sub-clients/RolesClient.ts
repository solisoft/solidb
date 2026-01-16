import type { Client } from '../Client';

export interface Permission {
    action: string;
    scope: string;
    database?: string;
}

export class RolesClient {
    constructor(private client: Client) {}

    async list(): Promise<any[]> {
        return (await this.client.sendCommand('list_roles', {})) || [];
    }

    async create(
        name: string,
        permissions: Permission[],
        description?: string
    ): Promise<any> {
        return this.client.sendCommand('create_role', {
            name,
            permissions,
            description
        });
    }

    async get(name: string): Promise<any> {
        return this.client.sendCommand('get_role', {
            role_name: name
        });
    }

    async update(
        name: string,
        permissions: Permission[],
        description?: string
    ): Promise<any> {
        return this.client.sendCommand('update_role', {
            role_name: name,
            permissions,
            description
        });
    }

    async delete(name: string): Promise<void> {
        await this.client.sendCommand('delete_role', {
            role_name: name
        });
    }
}
