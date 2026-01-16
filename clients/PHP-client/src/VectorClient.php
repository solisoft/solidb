<?php

namespace SoliDB;

class VectorClient
{
    private Client $client;

    public function __construct(Client $client)
    {
        $this->client = $client;
    }

    public function createIndex(string $collection, string $name, string $field, int $dimensions, ?string $metric = null, array $options = []): array
    {
        $args = [
            'database' => $this->client->getDatabase(),
            'collection' => $collection,
            'name' => $name,
            'field' => $field,
            'dimensions' => $dimensions
        ];
        if ($metric !== null) {
            $args['metric'] = $metric;
        }
        $args = array_merge($args, $options);
        $res = $this->client->sendCommand('create_vector_index', $args);
        return $res['data'] ?? [];
    }

    public function listIndexes(string $collection): array
    {
        $res = $this->client->sendCommand('list_vector_indexes', [
            'database' => $this->client->getDatabase(),
            'collection' => $collection
        ]);
        return $res['data'] ?? [];
    }

    public function deleteIndex(string $collection, string $indexName): void
    {
        $this->client->sendCommand('delete_vector_index', [
            'database' => $this->client->getDatabase(),
            'collection' => $collection,
            'index_name' => $indexName
        ]);
    }

    public function search(string $collection, array $vector, int $limit, ?array $filter = null): array
    {
        $args = [
            'database' => $this->client->getDatabase(),
            'collection' => $collection,
            'vector' => $vector,
            'limit' => $limit
        ];
        if ($filter !== null) {
            $args['filter'] = $filter;
        }
        $res = $this->client->sendCommand('vector_search', $args);
        return $res['data'] ?? [];
    }

    public function searchByDocument(string $collection, string $docKey, string $field, int $limit, ?array $filter = null): array
    {
        $args = [
            'database' => $this->client->getDatabase(),
            'collection' => $collection,
            'doc_key' => $docKey,
            'field' => $field,
            'limit' => $limit
        ];
        if ($filter !== null) {
            $args['filter'] = $filter;
        }
        $res = $this->client->sendCommand('vector_search_by_doc', $args);
        return $res['data'] ?? [];
    }

    public function quantize(string $collection, string $indexName, string $quantization): void
    {
        $this->client->sendCommand('vector_quantize', [
            'database' => $this->client->getDatabase(),
            'collection' => $collection,
            'index_name' => $indexName,
            'quantization' => $quantization
        ]);
    }

    public function dequantize(string $collection, string $indexName): void
    {
        $this->client->sendCommand('vector_dequantize', [
            'database' => $this->client->getDatabase(),
            'collection' => $collection,
            'index_name' => $indexName
        ]);
    }

    public function getIndexInfo(string $collection, string $indexName): array
    {
        $res = $this->client->sendCommand('vector_index_info', [
            'database' => $this->client->getDatabase(),
            'collection' => $collection,
            'index_name' => $indexName
        ]);
        return $res['data'] ?? [];
    }
}
