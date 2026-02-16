<?php

namespace SoliDB\Laravel\Connections;

use Illuminate\Database\Connection;
use SoliDB\Client as SoliDBClient;
use SoliDB\Laravel\Query\Builders\SoliDBBuilder;
use SoliDB\Laravel\Schema\Builders\SoliDBSchemaBuilder;

class SoliDBConnection extends Connection
{
    protected ?SoliDBClient $client = null;

    protected ?string $transactionId = null;

    public function __construct(array $config)
    {
        $this->config = $config;
        $this->database = $config['database'] ?? 'default';
        $this->tablePrefix = $config['prefix'] ?? '';
        $this->client = $this->createClient($config);

        $this->useDefaultQueryGrammar();
        $this->useDefaultPostProcessor();
    }

    protected function createClient(array $config): SoliDBClient
    {
        $client = new SoliDBClient(
            $config['host'] ?? '127.0.0.1',
            $config['port'] ?? 6745
        );

        $client->connect();

        if (!empty($config['username']) && !empty($config['password'])) {
            $client->auth(
                $this->database,
                $config['username'],
                $config['password']
            );
        } elseif (!empty($config['api_key'])) {
            $client->authWithApiKey($this->database, $config['api_key']);
        }

        return $client;
    }

    public function getDriverName(): string
    {
        return 'solidb';
    }

    public function select(string $query, array $bindings = [], bool $useReadPdo = true): array
    {
        $bindings = $this->formatBindings($bindings);

        $result = $this->client->query($this->database, $query, $bindings);

        return $result;
    }

    public function insert(string $query, array $bindings = []): bool
    {
        $bindings = $this->formatBindings($bindings);
        $this->client->query($this->database, $query, $bindings);
        return true;
    }

    public function update(string $query, array $bindings = []): int
    {
        $bindings = $this->formatBindings($bindings);
        $result = $this->client->query($this->database, $query, $bindings);
        return $result['updated'] ?? 0;
    }

    public function delete(string $query, array $bindings = []): int
    {
        $bindings = $this->formatBindings($bindings);
        $result = $this->client->query($this->database, $query, $bindings);
        return $result['deleted'] ?? 0;
    }

    public function statement(string $query, array $bindings = []): bool
    {
        $bindings = $this->formatBindings($bindings);
        $this->client->query($this->database, $query, $bindings);
        return true;
    }

    public function affectingStatement(string $query, array $bindings = []): int
    {
        $bindings = $this->formatBindings($bindings);
        $result = $this->client->query($this->database, $query, $bindings);
        return $result['updated'] ?? $result['deleted'] ?? 0;
    }

    public function unprepared(string $query): array
    {
        return $this->select($query, []);
    }

    protected function formatBindings(array $bindings): array
    {
        $formatted = [];
        foreach ($bindings as $key => $value) {
            if (is_int($key)) {
                $formatted[$key] = $value;
            } else {
                $formatted[$key] = $value;
            }
        }
        return $formatted;
    }

    public function transactionLevel(): int
    {
        return $this->transactions;
    }

    public function beginTransaction(): void
    {
        $this->transactions++;
        
        if ($this->transactions === 1) {
            $this->transactionId = $this->client->beginTransaction($this->database);
        }
    }

    public function commit(): void
    {
        if ($this->transactions === 1) {
            if ($this->transactionId) {
                $this->client->commitTransaction($this->transactionId);
                $this->transactionId = null;
            }
        }

        $this->transactions = max(0, $this->transactions - 1);
    }

    public function rollBack(int $toLevel = null): void
    {
        $toLevel = $toLevel ?? 0;

        if ($this->transactions >= $toLevel) {
            if ($this->transactionId) {
                $this->client->rollbackTransaction($this->transactionId);
                $this->transactionId = null;
            }
        }

        $this->transactions = $toLevel;
    }

    public function disconnect(): void
    {
        if ($this->client) {
            $this->client->close();
            $this->client = null;
        }
    }

    public function getClient(): SoliDBClient
    {
        return $this->client;
    }

    public function getSchemaBuilder()
    {
        if (is_null($this->schemaGrammar)) {
            $this->useDefaultSchemaGrammar();
        }

        return new SoliDBSchemaBuilder($this);
    }

    public function table($table, $as = null)
    {
        $query = new SoliDBBuilder(
            $this,
            $this->getQueryGrammar(),
            $this->getPostProcessor()
        );

        return $query->from($table);
    }

    public function getDatabaseName(): string
    {
        return $this->database;
    }

    public function setDatabaseName(string $database): self
    {
        $this->database = $database;
        $this->config['database'] = $database;
        
        if ($this->client) {
            $this->client->useDatabase($database);
        }
        
        return $this;
    }

    public function getServerVersion(): string
    {
        try {
            $latency = $this->client->ping();
            return '1.0.0';
        } catch (\Exception $e) {
            return 'unknown';
        }
    }
}
