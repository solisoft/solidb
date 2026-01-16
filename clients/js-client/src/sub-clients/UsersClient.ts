import type { Client } from '../Client';

export class UsersClient {
    constructor(private client: Client) {}

    async list(): Promise<any[]> {
        return (await this.client.sendCommand('list_users', {})) || [];
    }

    async create(username: string, password: string, roles?: string[]): Promise<any> {
        return this.client.sendCommand('create_user', {
            username,
            password,
            roles
        });
    }

    async get(username: string): Promise<any> {
        return this.client.sendCommand('get_user', {
            username
        });
    }

    async delete(username: string): Promise<void> {
        await this.client.sendCommand('delete_user', {
            username
        });
    }

    async getRoles(username: string): Promise<any[]> {
        return (await this.client.sendCommand('get_user_roles', {
            username
        })) || [];
    }

    async assignRole(username: string, role: string, database?: string): Promise<void> {
        await this.client.sendCommand('assign_role', {
            username,
            role,
            database
        });
    }

    async revokeRole(username: string, role: string, database?: string): Promise<void> {
        await this.client.sendCommand('revoke_role', {
            username,
            role,
            database
        });
    }

    async me(): Promise<any> {
        return this.client.sendCommand('get_current_user', {});
    }

    async myPermissions(): Promise<any[]> {
        return (await this.client.sendCommand('get_my_permissions', {})) || [];
    }

    async changePassword(username: string, oldPassword: string, newPassword: string): Promise<void> {
        await this.client.sendCommand('change_password', {
            username,
            old_password: oldPassword,
            new_password: newPassword
        });
    }
}
