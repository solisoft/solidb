"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.ClusterClient = void 0;
class ClusterClient {
    constructor(client) {
        this.client = client;
    }
    async status() {
        return this.client.sendCommand('cluster_status', {});
    }
    async info() {
        return this.client.sendCommand('cluster_info', {});
    }
    async removeNode(nodeId) {
        await this.client.sendCommand('cluster_remove_node', {
            node_id: nodeId
        });
    }
    async rebalance() {
        await this.client.sendCommand('cluster_rebalance', {});
    }
    async cleanup() {
        await this.client.sendCommand('cluster_cleanup', {});
    }
    async reshard(numShards) {
        await this.client.sendCommand('cluster_reshard', {
            num_shards: numShards
        });
    }
    async getNodes() {
        return (await this.client.sendCommand('cluster_get_nodes', {})) || [];
    }
    async getShards() {
        return (await this.client.sendCommand('cluster_get_shards', {})) || [];
    }
}
exports.ClusterClient = ClusterClient;
