import type { Client } from '../Client';

export class ApiKeysClient {
    constructor(private client: Client) {}

    async list(): Promise<any[]> {
        return (await this.client.sendCommand('list_api_keys', {})) || [];
    }

    async create(
        name: string,
        permissions: Array<{ action: string; scope: string; database?: string }>,
        expiresAt?: string
    ): Promise<any> {
        return this.client.sendCommand('create_api_key', {
            name,
            permissions,
            expires_at: expiresAt
        });
    }

    async get(keyId: string): Promise<any> {
        return this.client.sendCommand('get_api_key', {
            key_id: keyId
        });
    }

    async delete(keyId: string): Promise<void> {
        await this.client.sendCommand('delete_api_key', {
            key_id: keyId
        });
    }

    async regenerate(keyId: string): Promise<any> {
        return this.client.sendCommand('regenerate_api_key', {
            key_id: keyId
        });
    }
}
