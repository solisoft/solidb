import type { Client } from '../Client';

export class ClusterClient {
    constructor(private client: Client) {}

    async status(): Promise<any> {
        return this.client.sendCommand('cluster_status', {});
    }

    async info(): Promise<any> {
        return this.client.sendCommand('cluster_info', {});
    }

    async removeNode(nodeId: string): Promise<void> {
        await this.client.sendCommand('cluster_remove_node', {
            node_id: nodeId
        });
    }

    async rebalance(): Promise<void> {
        await this.client.sendCommand('cluster_rebalance', {});
    }

    async cleanup(): Promise<void> {
        await this.client.sendCommand('cluster_cleanup', {});
    }

    async reshard(numShards: number): Promise<void> {
        await this.client.sendCommand('cluster_reshard', {
            num_shards: numShards
        });
    }

    async getNodes(): Promise<any[]> {
        return (await this.client.sendCommand('cluster_get_nodes', {})) || [];
    }

    async getShards(): Promise<any[]> {
        return (await this.client.sendCommand('cluster_get_shards', {})) || [];
    }
}
