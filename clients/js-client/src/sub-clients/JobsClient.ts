import type { Client } from '../Client';

export class JobsClient {
    constructor(private client: Client) {}

    async listQueues(): Promise<any[]> {
        return (await this.client.sendCommand('list_queues', {
            database: this.client.database
        })) || [];
    }

    async listJobs(
        queueName: string,
        options?: { status?: string; limit?: number; offset?: number }
    ): Promise<any[]> {
        return (await this.client.sendCommand('list_jobs', {
            database: this.client.database,
            queue_name: queueName,
            ...options
        })) || [];
    }

    async enqueue(
        queueName: string,
        scriptPath: string,
        options?: { params?: Record<string, any>; priority?: number; runAt?: string }
    ): Promise<any> {
        return this.client.sendCommand('enqueue_job', {
            database: this.client.database,
            queue_name: queueName,
            script_path: scriptPath,
            ...options
        });
    }

    async cancel(jobId: string): Promise<void> {
        await this.client.sendCommand('cancel_job', {
            database: this.client.database,
            job_id: jobId
        });
    }

    async get(jobId: string): Promise<any> {
        return this.client.sendCommand('get_job', {
            database: this.client.database,
            job_id: jobId
        });
    }
}
