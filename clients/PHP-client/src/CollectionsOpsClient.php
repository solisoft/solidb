<?php

namespace SoliDB;

class CollectionsOpsClient
{
    private Client $client;

    public function __construct(Client $client)
    {
        $this->client = $client;
    }

    public function truncate(string $collection): void
    {
        $this->client->sendCommand('truncate_collection', [
            'database' => $this->client->getDatabase(),
            'collection' => $collection
        ]);
    }

    public function compact(string $collection): void
    {
        $this->client->sendCommand('compact_collection', [
            'database' => $this->client->getDatabase(),
            'collection' => $collection
        ]);
    }

    public function stats(string $collection): array
    {
        $res = $this->client->sendCommand('collection_stats', [
            'database' => $this->client->getDatabase(),
            'collection' => $collection
        ]);
        return $res['data'] ?? [];
    }

    public function prune(string $collection, ?string $olderThan = null, ?string $field = null): array
    {
        $args = [
            'database' => $this->client->getDatabase(),
            'collection' => $collection
        ];
        if ($olderThan !== null) {
            $args['older_than'] = $olderThan;
        }
        if ($field !== null) {
            $args['field'] = $field;
        }
        $res = $this->client->sendCommand('prune_collection', $args);
        return $res['data'] ?? [];
    }

    public function recount(string $collection): array
    {
        $res = $this->client->sendCommand('recount_collection', [
            'database' => $this->client->getDatabase(),
            'collection' => $collection
        ]);
        return $res['data'] ?? [];
    }

    public function repair(string $collection): array
    {
        $res = $this->client->sendCommand('repair_collection', [
            'database' => $this->client->getDatabase(),
            'collection' => $collection
        ]);
        return $res['data'] ?? [];
    }

    public function setSchema(string $collection, array $schema): void
    {
        $this->client->sendCommand('set_collection_schema', [
            'database' => $this->client->getDatabase(),
            'collection' => $collection,
            'schema' => $schema
        ]);
    }

    public function getSchema(string $collection): array
    {
        $res = $this->client->sendCommand('get_collection_schema', [
            'database' => $this->client->getDatabase(),
            'collection' => $collection
        ]);
        return $res['data'] ?? [];
    }

    public function deleteSchema(string $collection): void
    {
        $this->client->sendCommand('delete_collection_schema', [
            'database' => $this->client->getDatabase(),
            'collection' => $collection
        ]);
    }

    public function export(string $collection, string $format): mixed
    {
        $res = $this->client->sendCommand('export_collection', [
            'database' => $this->client->getDatabase(),
            'collection' => $collection,
            'format' => $format
        ]);
        return $res['data'] ?? null;
    }

    public function import(string $collection, mixed $data, string $format): array
    {
        $res = $this->client->sendCommand('import_collection', [
            'database' => $this->client->getDatabase(),
            'collection' => $collection,
            'data' => $data,
            'format' => $format
        ]);
        return $res['data'] ?? [];
    }

    public function getSharding(string $collection): array
    {
        $res = $this->client->sendCommand('get_collection_sharding', [
            'database' => $this->client->getDatabase(),
            'collection' => $collection
        ]);
        return $res['data'] ?? [];
    }

    public function setSharding(string $collection, array $config): void
    {
        $args = array_merge([
            'database' => $this->client->getDatabase(),
            'collection' => $collection
        ], $config);
        $this->client->sendCommand('set_collection_sharding', $args);
    }
}
