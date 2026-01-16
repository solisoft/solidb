import type { Client } from '../Client';

export class TriggersClient {
    constructor(private client: Client) {}

    async list(): Promise<any[]> {
        return (await this.client.sendCommand('list_triggers', {
            database: this.client.database
        })) || [];
    }

    async listByCollection(collection: string): Promise<any[]> {
        return (await this.client.sendCommand('list_triggers_by_collection', {
            database: this.client.database,
            collection
        })) || [];
    }

    async create(
        name: string,
        collection: string,
        event: string,
        timing: string,
        scriptPath: string,
        enabled?: boolean
    ): Promise<any> {
        return this.client.sendCommand('create_trigger', {
            database: this.client.database,
            name,
            collection,
            event,
            timing,
            script_path: scriptPath,
            enabled
        });
    }

    async get(triggerId: string): Promise<any> {
        return this.client.sendCommand('get_trigger', {
            database: this.client.database,
            trigger_id: triggerId
        });
    }

    async update(triggerId: string, updates: Record<string, any>): Promise<any> {
        return this.client.sendCommand('update_trigger', {
            database: this.client.database,
            trigger_id: triggerId,
            updates
        });
    }

    async delete(triggerId: string): Promise<void> {
        await this.client.sendCommand('delete_trigger', {
            database: this.client.database,
            trigger_id: triggerId
        });
    }

    async toggle(triggerId: string, enabled: boolean): Promise<void> {
        await this.client.sendCommand('toggle_trigger', {
            database: this.client.database,
            trigger_id: triggerId,
            enabled
        });
    }
}
