"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.CronClient = void 0;
class CronClient {
    constructor(client) {
        this.client = client;
    }
    async list() {
        return (await this.client.sendCommand('list_cron_jobs', {
            database: this.client.database
        })) || [];
    }
    async create(name, schedule, scriptPath, options) {
        return this.client.sendCommand('create_cron_job', {
            database: this.client.database,
            name,
            schedule,
            script_path: scriptPath,
            ...options
        });
    }
    async get(cronId) {
        return this.client.sendCommand('get_cron_job', {
            database: this.client.database,
            cron_id: cronId
        });
    }
    async update(cronId, updates) {
        return this.client.sendCommand('update_cron_job', {
            database: this.client.database,
            cron_id: cronId,
            updates
        });
    }
    async delete(cronId) {
        await this.client.sendCommand('delete_cron_job', {
            database: this.client.database,
            cron_id: cronId
        });
    }
    async toggle(cronId, enabled) {
        await this.client.sendCommand('toggle_cron_job', {
            database: this.client.database,
            cron_id: cronId,
            enabled
        });
    }
}
exports.CronClient = CronClient;
