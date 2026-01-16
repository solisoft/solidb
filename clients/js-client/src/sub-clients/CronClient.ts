import type { Client } from '../Client';

export class CronClient {
    constructor(private client: Client) {}

    async list(): Promise<any[]> {
        return (await this.client.sendCommand('list_cron_jobs', {
            database: this.client.database
        })) || [];
    }

    async create(
        name: string,
        schedule: string,
        scriptPath: string,
        options?: { params?: Record<string, any>; enabled?: boolean; description?: string }
    ): Promise<any> {
        return this.client.sendCommand('create_cron_job', {
            database: this.client.database,
            name,
            schedule,
            script_path: scriptPath,
            ...options
        });
    }

    async get(cronId: string): Promise<any> {
        return this.client.sendCommand('get_cron_job', {
            database: this.client.database,
            cron_id: cronId
        });
    }

    async update(cronId: string, updates: Record<string, any>): Promise<any> {
        return this.client.sendCommand('update_cron_job', {
            database: this.client.database,
            cron_id: cronId,
            updates
        });
    }

    async delete(cronId: string): Promise<void> {
        await this.client.sendCommand('delete_cron_job', {
            database: this.client.database,
            cron_id: cronId
        });
    }

    async toggle(cronId: string, enabled: boolean): Promise<void> {
        await this.client.sendCommand('toggle_cron_job', {
            database: this.client.database,
            cron_id: cronId,
            enabled
        });
    }
}
