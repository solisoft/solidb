import type { Client } from '../Client';

export class EnvClient {
    constructor(private client: Client) {}

    async list(): Promise<Record<string, any>> {
        return (await this.client.sendCommand('list_env_vars', {
            database: this.client.database
        })) || {};
    }

    async get(key: string): Promise<any> {
        return this.client.sendCommand('get_env_var', {
            database: this.client.database,
            key
        });
    }

    async set(key: string, value: any): Promise<void> {
        await this.client.sendCommand('set_env_var', {
            database: this.client.database,
            key,
            value
        });
    }

    async delete(key: string): Promise<void> {
        await this.client.sendCommand('delete_env_var', {
            database: this.client.database,
            key
        });
    }

    async setBulk(vars: Record<string, any>): Promise<void> {
        await this.client.sendCommand('set_env_vars_bulk', {
            database: this.client.database,
            vars
        });
    }
}
