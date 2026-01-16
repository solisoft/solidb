<?php

namespace SoliDB;

class ColumnarClient
{
    private Client $client;

    public function __construct(Client $client)
    {
        $this->client = $client;
    }

    public function create(string $name, array $columns): array
    {
        $res = $this->client->sendCommand('create_columnar_table', [
            'database' => $this->client->getDatabase(),
            'name' => $name,
            'columns' => $columns
        ]);
        return $res['data'] ?? [];
    }

    public function list(): array
    {
        $res = $this->client->sendCommand('list_columnar_tables', ['database' => $this->client->getDatabase()]);
        return $res['data'] ?? [];
    }

    public function get(string $name): array
    {
        $res = $this->client->sendCommand('get_columnar_table', [
            'database' => $this->client->getDatabase(),
            'name' => $name
        ]);
        return $res['data'] ?? [];
    }

    public function delete(string $name): void
    {
        $this->client->sendCommand('delete_columnar_table', [
            'database' => $this->client->getDatabase(),
            'name' => $name
        ]);
    }

    public function insert(string $name, array $rows): array
    {
        $res = $this->client->sendCommand('columnar_insert', [
            'database' => $this->client->getDatabase(),
            'name' => $name,
            'rows' => $rows
        ]);
        return $res['data'] ?? [];
    }

    public function query(string $name, string $query, ?array $params = null): array
    {
        $args = [
            'database' => $this->client->getDatabase(),
            'name' => $name,
            'query' => $query
        ];
        if ($params !== null) {
            $args['params'] = $params;
        }
        $res = $this->client->sendCommand('columnar_query', $args);
        return $res['data'] ?? [];
    }

    public function aggregate(string $name, array $aggregation): array
    {
        $res = $this->client->sendCommand('columnar_aggregate', [
            'database' => $this->client->getDatabase(),
            'name' => $name,
            'aggregation' => $aggregation
        ]);
        return $res['data'] ?? [];
    }

    public function createIndex(string $tableName, string $indexName, string $column, ?string $indexType = null): array
    {
        $args = [
            'database' => $this->client->getDatabase(),
            'table_name' => $tableName,
            'index_name' => $indexName,
            'column' => $column
        ];
        if ($indexType !== null) {
            $args['index_type'] = $indexType;
        }
        $res = $this->client->sendCommand('columnar_create_index', $args);
        return $res['data'] ?? [];
    }

    public function listIndexes(string $tableName): array
    {
        $res = $this->client->sendCommand('columnar_list_indexes', [
            'database' => $this->client->getDatabase(),
            'table_name' => $tableName
        ]);
        return $res['data'] ?? [];
    }

    public function deleteIndex(string $tableName, string $indexName): void
    {
        $this->client->sendCommand('columnar_delete_index', [
            'database' => $this->client->getDatabase(),
            'table_name' => $tableName,
            'index_name' => $indexName
        ]);
    }

    public function addColumn(string $tableName, string $columnName, string $columnType, mixed $defaultValue = null): void
    {
        $args = [
            'database' => $this->client->getDatabase(),
            'table_name' => $tableName,
            'column_name' => $columnName,
            'column_type' => $columnType
        ];
        if ($defaultValue !== null) {
            $args['default_value'] = $defaultValue;
        }
        $this->client->sendCommand('columnar_add_column', $args);
    }

    public function dropColumn(string $tableName, string $columnName): void
    {
        $this->client->sendCommand('columnar_drop_column', [
            'database' => $this->client->getDatabase(),
            'table_name' => $tableName,
            'column_name' => $columnName
        ]);
    }

    public function stats(string $tableName): array
    {
        $res = $this->client->sendCommand('columnar_stats', [
            'database' => $this->client->getDatabase(),
            'table_name' => $tableName
        ]);
        return $res['data'] ?? [];
    }
}
