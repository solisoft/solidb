<?php

namespace SoliDB\Laravel\Query\Grammars;

use Illuminate\Database\Grammar as BaseGrammar;
use Illuminate\Database\Query\Grammars\Grammar as QueryGrammar;

class Grammar extends QueryGrammar
{
    public function __construct($connection = null)
    {
        parent::__construct();
    }

    protected $selectComponents = [
        'aggregate',
        'columns',
        'from',
        'joins',
        'wheres',
        'groups',
        'havings',
        'orders',
        'limit',
        'offset',
        'lock',
    ];

    public function compileSelect($query): string
    {
        if ($query->aggregate) {
            return $this->compileAggregate($query, $query->aggregate['function'], $query->aggregate['columns']);
        }

        $sql = $this->compileColumns($query);

        if ($query->from) {
            $sql .= ' ' . $this->compileFrom($query);
        }

        if ($query->joins) {
            $sql .= ' ' . $this->compileJoins($query, $query->joins);
        }

        if ($query->wheres) {
            $sql .= ' ' . $this->compileWheres($query);
        }

        if ($query->groups) {
            $sql .= ' ' . $this->compileGroups($query);
        }

        if ($query->havings) {
            $sql .= ' ' . $this->compileHavings($query);
        }

        if ($query->orders) {
            $sql .= ' ' . $this->compileOrders($query);
        }

        if ($query->limit) {
            $sql .= ' ' . $this->compileLimit($query);
        }

        if ($query->offset) {
            $sql .= ' ' . $this->compileOffset($query);
        }

        return trim($sql);
    }

    protected function compileColumns($query): string
    {
        if (!$query->columns) {
            return 'RETURN {}';
        }

        $columns = array_map(function ($column) {
            if (is_string($column)) {
                return $column;
            }
            if ($column instanceof \Illuminate\Database\Query\Expression) {
                return $column->getValue();
            }
            return $column;
        }, $query->columns);

        if (in_array('*', $columns)) {
            return 'RETURN doc';
        }

        return 'RETURN ' . implode(', ', $columns);
    }

    protected function compileFrom($query): string
    {
        $from = $this->wrapTable($query->from);

        if ($query->from !== ($query->fromAlias ?? $query->from)) {
            $from .= ' AS ' . $this->wrap($query->fromAlias ?? $query->from);
        }

        return "FOR doc IN {$from}";
    }

    protected function compileWheres($query): string
    {
        $wheres = [];

        foreach ($query->wheres as $where) {
            $method = 'where' . ucfirst($where['type']);
            $wheres[] = $this->{$method}($query, $where);
        }

        if (!empty($wheres)) {
            return 'FILTER ' . implode(' AND ', $wheres);
        }

        return '';
    }

    protected function whereBasic($query, $where): string
    {
        $column = $this->wrap($where['column']);
        $operator = $where['operator'];
        $value = $this->parameter($where['value']);

        return "{$column} {$operator} {$value}";
    }

    protected function whereNull($query, $where): string
    {
        return $this->wrap($where['column']) . ' == null';
    }

    protected function whereNotNull($query, $where): string
    {
        return $this->wrap($where['column']) . ' != null';
    }

    protected function whereIn($query, $where): string
    {
        $values = array_map([$this, 'parameter'], $where['values']);
        return $this->wrap($where['column']) . ' IN [' . implode(', ', $values) . ']';
    }

    protected function whereNotIn($query, $where): string
    {
        $values = array_map([$this, 'parameter'], $where['values']);
        return $this->wrap($where['column']) . ' NOT IN [' . implode(', ', $values) . ']';
    }

    protected function whereBetween($query, $where): string
    {
        $not = $where['not'] ? 'NOT ' : '';
        return $not . $this->wrap($where['column']) . ' BETWEEN ' . 
               $this->parameter($where['values'][0]) . ' AND ' . $this->parameter($where['values'][1]);
    }

    protected function whereDate($query, $where): string
    {
        return $this->whereBasic($query, $where);
    }

    protected function whereTime($query, $where): string
    {
        return $this->whereBasic($query, $where);
    }

    protected function whereDay($query, $where): string
    {
        return $this->whereBasic($query, $where);
    }

    protected function whereMonth($query, $where): string
    {
        return $this->whereBasic($query, $where);
    }

    protected function whereYear($query, $where): string
    {
        return $this->whereBasic($query, $where);
    }

    protected function whereRaw($query, $where): string
    {
        return $where['sql'];
    }

    protected function compileGroups($query): string
    {
        return 'GROUP BY ' . implode(', ', array_map([$this, 'wrap'], $query->groups));
    }

    protected function compileHavings($query): string
    {
        $sql = 'HAVING ';

        foreach ($query->havings as $i => $having) {
            if ($i > 0) {
                $sql .= ($having['boolean'] ?? 'AND') . ' ';
            }

            if ($having['type'] === 'Raw') {
                $sql .= $having['sql'];
            } else {
                $column = $this->wrap($having['column']);
                $operator = $having['operator'];
                $value = $this->parameter($having['value']);
                $sql .= "{$column} {$operator} {$value}";
            }
        }

        return $sql;
    }

    protected function compileOrders($query): string
    {
        if (empty($query->orders)) {
            return '';
        }

        return 'SORT ' . implode(', ', array_map(function ($order) {
            $direction = $order['direction'] ?? 'ASC';
            return $this->wrap($order['column']) . ' ' . strtoupper($direction);
        }, $query->orders));
    }

    protected function compileLimit($query): string
    {
        return 'LIMIT ' . (int) $query->limit;
    }

    protected function compileOffset($query): string
    {
        return 'LIMIT ' . (int) $query->offset . ', ' . ($query->limit ?? 100);
    }

    public function compileAggregate($query, $function, $columns): string
    {
        $column = $columns[0] ?? '*';
        
        if ($column !== '*') {
            $column = $this->wrap($column);
        }

        $aggregate = "{$function}({$column}) AS aggregate";

        return $this->compileSelect($query) . ' RETURN {' . $aggregate . '}';
    }

    public function compileInsert($query, $values): string
    {
        $table = $this->wrapTable($query->from);
        
        $keys = array_keys($values);
        $fields = implode(', ', array_map([$this, 'wrap'], $keys));
        
        $placeholders = implode(', ', array_fill(0, count($keys), '?'));
        
        return "INSERT {{$fields}: [{$placeholders}]}} INTO {$table}";
    }

    public function compileUpdate($query, $values): string
    {
        $table = $this->wrapTable($query->from);
        
        $sets = [];
        foreach ($values as $key => $value) {
            $sets[] = $this->wrap($key) . ': ' . $this->parameter($value);
        }
        
        $sql = 'UPDATE ' . $table . ' SET {' . implode(', ', $sets) . '}';
        
        if (!empty($query->wheres)) {
            $sql .= ' ' . $this->compileWheres($query);
        }
        
        return $sql;
    }

    public function compileDelete($query): string
    {
        $table = $this->wrapTable($query->from);
        $sql = 'REMOVE doc IN ' . $table;
        
        if (!empty($query->wheres)) {
            $sql .= ' ' . $this->compileWheres($query);
        }
        
        return $sql;
    }

    protected function compileJoins($query, $joins): string
    {
        $sql = '';
        
        foreach ($joins as $join) {
            $type = $join['type'] ?? 'INNER';
            $table = $this->wrapTable($join['table']);
            $alias = $join['alias'] ?? '';
            
            if ($alias) {
                $table .= ' AS ' . $alias;
            }
            
            $sql .= strtoupper($type) . " JOIN {$table}";
            
            if ($join['clauses']) {
                $onClauses = [];
                foreach ($join['clauses'] as $clause) {
                    $first = $this->wrap($clause['first']);
                    $operator = $clause['operator'];
                    $second = $this->wrap($clause['second']);
                    $boolean = $clause['boolean'] ?? 'AND';
                    $onClauses[] = "{$boolean} {$first} {$operator} {$second}";
                }
                $sql .= ' ON ' . ltrim(implode(' ', $onClauses), 'AND ');
            }
        }
        
        return $sql;
    }

    public function wrap($value): string
    {
        if ($value instanceof \Illuminate\Database\Query\Expression) {
            return $value->getValue();
        }

        if ($value === '*') {
            return $value;
        }

        if (strpos($value, '.') !== false) {
            $parts = explode('.', $value);
            return implode('.', array_map([$this, 'wrapSegment'], $parts));
        }

        return '`' . $value . '`';
    }

    protected function wrapSegment(string $value): string
    {
        return '`' . $value . '`';
    }

    public function wrapTable($table): string
    {
        if ($table instanceof \Illuminate\Database\Query\Expression) {
            return $table->getValue();
        }

        return $this->wrap($table);
    }

    public function parameter($value): string
    {
        if ($value instanceof \Illuminate\Database\Query\Expression) {
            return $value->getValue();
        }

        if (is_null($value)) {
            return 'null';
        }

        if (is_bool($value)) {
            return $value ? 'true' : 'false';
        }

        if (is_int($value) || is_float($value)) {
            return (string) $value;
        }

        if (is_array($value)) {
            if (array_is_list($value)) {
                return '[' . implode(', ', array_map([$this, 'parameter'], $value)) . ']';
            }
            $pairs = [];
            foreach ($value as $k => $v) {
                $pairs[] = $this->wrap($k) . ': ' . $this->parameter($v);
            }
            return '{' . implode(', ', $pairs) . '}';
        }

        return '"' . addcslashes((string) $value, '"\\') . '"';
    }

    protected function getBindings(): array
    {
        return [];
    }
}
