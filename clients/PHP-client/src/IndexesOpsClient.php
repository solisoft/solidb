<?php

namespace SoliDB;

class IndexesOpsClient
{
    private Client $client;

    public function __construct(Client $client)
    {
        $this->client = $client;
    }

    public function rebuild(string $collection, string $indexName): void
    {
        $this->client->sendCommand('rebuild_index', [
            'database' => $this->client->getDatabase(),
            'collection' => $collection,
            'index_name' => $indexName
        ]);
    }

    public function rebuildAll(string $collection): void
    {
        $this->client->sendCommand('rebuild_all_indexes', [
            'database' => $this->client->getDatabase(),
            'collection' => $collection
        ]);
    }

    public function hybridSearch(string $collection, array $query): array
    {
        $args = array_merge([
            'database' => $this->client->getDatabase(),
            'collection' => $collection
        ], $query);
        $res = $this->client->sendCommand('hybrid_search', $args);
        return $res['data'] ?? [];
    }

    public function analyze(string $collection, string $indexName): array
    {
        $res = $this->client->sendCommand('analyze_index', [
            'database' => $this->client->getDatabase(),
            'collection' => $collection,
            'index_name' => $indexName
        ]);
        return $res['data'] ?? [];
    }

    public function getUsageStats(string $collection): array
    {
        $res = $this->client->sendCommand('index_usage_stats', [
            'database' => $this->client->getDatabase(),
            'collection' => $collection
        ]);
        return $res['data'] ?? [];
    }
}
