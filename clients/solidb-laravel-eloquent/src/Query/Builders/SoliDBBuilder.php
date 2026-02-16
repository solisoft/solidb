<?php

namespace SoliDB\Laravel\Query\Builders;

use Illuminate\Database\Query\Builder as BaseBuilder;

class SoliDBBuilder extends BaseBuilder
{
    public function insert(array $values): bool
    {
        if (empty($values)) {
            return true;
        }

        if (!is_array(reset($values))) {
            $values = [$values];
        }

        foreach ($values as $record) {
            $this->connection->insert(
                $this->grammar->compileInsert($this, $record),
                $this->cleanBindings($record)
            );
        }

        return true;
    }

    public function update(array $values): int
    {
        $sql = $this->grammar->compileUpdate($this, $values);
        $bindings = $this->cleanBindings(
            array_merge($values, $this->bindings['where'])
        );
        
        return $this->connection->update($sql, $bindings);
    }

    public function delete($id = null): int
    {
        if (!is_null($id)) {
            return $this->where('key', '=', $id)->delete();
        }

        $sql = $this->grammar->compileDelete($this);
        $bindings = $this->cleanBindings($this->bindings['where']);
        
        return $this->connection->delete($sql, $bindings);
    }

    public function exists(): bool
    {
        $results = $this->limit(1)->get();
        return !$results->isEmpty();
    }

    public function count($columns = ['*']): int
    {
        return (int) $this->aggregate(__FUNCTION__, $columns);
    }

    public function aggregate(string $function, array $columns = ['*'])
    {
        $this->bindings['select'] = [];
        
        $results = $this->connection->select(
            $this->grammar->compileAggregate($this, $function, $columns),
            $this->getBindings()
        );

        if (!empty($results)) {
            $result = (array) $results[0];
            $aggregate = array_change_key_case($result, CASE_LOWER);
            $key = strtolower($function) . '_' . reset($columns);
            
            if (isset($aggregate[$key])) {
                return $aggregate[$key];
            }
            
            if (isset($aggregate['aggregate'])) {
                return $aggregate['aggregate'];
            }
        }

        return null;
    }

    public function useDatabase(string $database): self
    {
        $this->connection->setDatabaseName($database);
        return $this;
    }
}
