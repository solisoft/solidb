"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.JobsClient = void 0;
class JobsClient {
    constructor(client) {
        this.client = client;
    }
    async listQueues() {
        return (await this.client.sendCommand('list_queues', {
            database: this.client.database
        })) || [];
    }
    async listJobs(queueName, options) {
        return (await this.client.sendCommand('list_jobs', {
            database: this.client.database,
            queue_name: queueName,
            ...options
        })) || [];
    }
    async enqueue(queueName, scriptPath, options) {
        return this.client.sendCommand('enqueue_job', {
            database: this.client.database,
            queue_name: queueName,
            script_path: scriptPath,
            ...options
        });
    }
    async cancel(jobId) {
        await this.client.sendCommand('cancel_job', {
            database: this.client.database,
            job_id: jobId
        });
    }
    async get(jobId) {
        return this.client.sendCommand('get_job', {
            database: this.client.database,
            job_id: jobId
        });
    }
}
exports.JobsClient = JobsClient;
