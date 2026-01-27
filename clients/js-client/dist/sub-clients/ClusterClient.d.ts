import type { Client } from '../Client';
export declare class ClusterClient {
    private client;
    constructor(client: Client);
    status(): Promise<any>;
    info(): Promise<any>;
    removeNode(nodeId: string): Promise<void>;
    rebalance(): Promise<void>;
    cleanup(): Promise<void>;
    reshard(numShards: number): Promise<void>;
    getNodes(): Promise<any[]>;
    getShards(): Promise<any[]>;
}
