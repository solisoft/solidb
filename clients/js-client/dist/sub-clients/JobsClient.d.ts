import type { Client } from '../Client';
export declare class JobsClient {
    private client;
    constructor(client: Client);
    listQueues(): Promise<any[]>;
    listJobs(queueName: string, options?: {
        status?: string;
        limit?: number;
        offset?: number;
    }): Promise<any[]>;
    enqueue(queueName: string, scriptPath: string, options?: {
        params?: Record<string, any>;
        priority?: number;
        runAt?: string;
    }): Promise<any>;
    cancel(jobId: string): Promise<void>;
    get(jobId: string): Promise<any>;
}
