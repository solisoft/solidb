<?php

namespace SoliDB;

class ClusterClient
{
    private Client $client;

    public function __construct(Client $client)
    {
        $this->client = $client;
    }

    public function status(): array
    {
        $res = $this->client->sendCommand('cluster_status', []);
        return $res['data'] ?? [];
    }

    public function info(): array
    {
        $res = $this->client->sendCommand('cluster_info', []);
        return $res['data'] ?? [];
    }

    public function removeNode(string $nodeId): void
    {
        $this->client->sendCommand('cluster_remove_node', ['node_id' => $nodeId]);
    }

    public function rebalance(): void
    {
        $this->client->sendCommand('cluster_rebalance', []);
    }

    public function cleanup(): void
    {
        $this->client->sendCommand('cluster_cleanup', []);
    }

    public function reshard(int $numShards): void
    {
        $this->client->sendCommand('cluster_reshard', ['num_shards' => $numShards]);
    }

    public function getNodes(): array
    {
        $res = $this->client->sendCommand('cluster_get_nodes', []);
        return $res['data'] ?? [];
    }

    public function getShards(): array
    {
        $res = $this->client->sendCommand('cluster_get_shards', []);
        return $res['data'] ?? [];
    }
}
