<?php

namespace SoliDB;

class TTLClient
{
    private Client $client;

    public function __construct(Client $client)
    {
        $this->client = $client;
    }

    public function createIndex(string $collection, string $name, string $field, int $expireAfterSeconds): array
    {
        $res = $this->client->sendCommand('create_ttl_index', [
            'database' => $this->client->getDatabase(),
            'collection' => $collection,
            'name' => $name,
            'field' => $field,
            'expire_after_seconds' => $expireAfterSeconds
        ]);
        return $res['data'] ?? [];
    }

    public function listIndexes(string $collection): array
    {
        $res = $this->client->sendCommand('list_ttl_indexes', [
            'database' => $this->client->getDatabase(),
            'collection' => $collection
        ]);
        return $res['data'] ?? [];
    }

    public function deleteIndex(string $collection, string $indexName): void
    {
        $this->client->sendCommand('delete_ttl_index', [
            'database' => $this->client->getDatabase(),
            'collection' => $collection,
            'index_name' => $indexName
        ]);
    }

    public function updateExpiration(string $collection, string $indexName, int $expireAfterSeconds): void
    {
        $this->client->sendCommand('update_ttl_expiration', [
            'database' => $this->client->getDatabase(),
            'collection' => $collection,
            'index_name' => $indexName,
            'expire_after_seconds' => $expireAfterSeconds
        ]);
    }

    public function getIndexInfo(string $collection, string $indexName): array
    {
        $res = $this->client->sendCommand('ttl_index_info', [
            'database' => $this->client->getDatabase(),
            'collection' => $collection,
            'index_name' => $indexName
        ]);
        return $res['data'] ?? [];
    }

    public function runCleanup(string $collection): array
    {
        $res = $this->client->sendCommand('ttl_run_cleanup', [
            'database' => $this->client->getDatabase(),
            'collection' => $collection
        ]);
        return $res['data'] ?? [];
    }
}
