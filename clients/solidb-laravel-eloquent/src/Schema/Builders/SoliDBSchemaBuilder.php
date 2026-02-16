<?php

namespace SoliDB\Laravel\Schema\Builders;

use Illuminate\Database\Schema\Builder as BaseBuilder;

class SoliDBSchemaBuilder extends BaseBuilder
{
    public function hasTable(string $table): bool
    {
        $tables = $this->connection->select('RETURN COLLECTION_NAMES()');
        
        if (is_array($tables) && !empty($tables)) {
            $firstTable = is_array($tables[0]) ? ($tables[0]['name'] ?? null) : $tables[0];
            return in_array($table, array_column($tables, 'name') ?: [$firstTable]);
        }
        
        return false;
    }

    public function getColumnListing(string $table): array
    {
        $result = $this->connection->select(
            "FOR doc IN {$table} LIMIT 1 RETURN MERGE(doc, {__key: doc._key})"
        );
        
        if (empty($result)) {
            return [];
        }
        
        return array_keys((array) $result[0]);
    }

    public function create(string $table, \Closure $callback): void
    {
        $blueprint = $this->createBlueprint($table, $callback);
        $blueprint->create();
    }

    public function drop(string $table): void
    {
        $this->connection->statement("REMOVE {__key: '{$table}'} IN _collections");
    }

    public function dropIfExists(string $table): void
    {
        if ($this->hasTable($table)) {
            $this->drop($table);
        }
    }

    public function dropAllTables(): void
    {
        $collections = $this->connection->select('RETURN COLLECTION_NAMES()');
        
        if (!empty($collections)) {
            foreach ($collections as $collection) {
                $name = is_array($collection) ? ($collection['name'] ?? '') : $collection;
                if ($name) {
                    $this->drop($name);
                }
            }
        }
    }

    public function enableForeignKeyConstraints(): void
    {
        // SoliDB doesn't support foreign keys
    }

    public function disableForeignKeyConstraints(): void
    {
        // SoliDB doesn't support foreign keys
    }

    protected function createBlueprint($table, \Closure $callback = null)
    {
        return new Blueprint($this->connection, $table, $callback);
    }
}
