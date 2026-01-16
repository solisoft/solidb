import type { Client } from '../Client';

export class ScriptsClient {
    constructor(private client: Client) {}

    async create(
        name: string,
        path: string,
        methods: string[],
        code: string,
        options?: { description?: string; collection?: string }
    ): Promise<any> {
        return this.client.sendCommand('create_script', {
            database: this.client.database,
            name,
            path,
            methods,
            code,
            ...options
        });
    }

    async list(): Promise<any[]> {
        return (await this.client.sendCommand('list_scripts', {
            database: this.client.database
        })) || [];
    }

    async get(scriptId: string): Promise<any> {
        return this.client.sendCommand('get_script', {
            database: this.client.database,
            script_id: scriptId
        });
    }

    async update(scriptId: string, updates: Record<string, any>): Promise<any> {
        return this.client.sendCommand('update_script', {
            database: this.client.database,
            script_id: scriptId,
            updates
        });
    }

    async delete(scriptId: string): Promise<void> {
        await this.client.sendCommand('delete_script', {
            database: this.client.database,
            script_id: scriptId
        });
    }

    async getStats(): Promise<any> {
        return this.client.sendCommand('get_script_stats', {});
    }
}
