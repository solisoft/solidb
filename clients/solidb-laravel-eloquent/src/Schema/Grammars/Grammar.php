<?php

namespace SoliDB\Laravel\Schema\Grammars;

use Illuminate\Database\Schema\Grammars\Grammar as BaseGrammar;

class Grammar extends BaseGrammar
{
    public function compileCreate($blueprint): string
    {
        $table = $this->wrapTable($blueprint);
        
        return "CREATE COLLECTION {$table}";
    }

    public function compileDrop($blueprint): string
    {
        $table = $this->wrapTable($blueprint);
        
        return "DROP COLLECTION {$table}";
    }

    public function compileDropIfExists($blueprint): string
    {
        return $this->compileDrop($blueprint);
    }

    protected function typeString($column): string
    {
        return 'string';
    }

    protected function typeText($column): string
    {
        return 'string';
    }

    protected function typeInteger($column): string
    {
        return 'integer';
    }

    protected function typeBigInteger($column): string
    {
        return 'integer';
    }

    protected function typeFloat($column): string
    {
        return 'float';
    }

    protected function typeDouble($column): string
    {
        return 'float';
    }

    protected function typeDecimal($column): string
    {
        return 'float';
    }

    protected function typeBoolean($column): string
    {
        return 'boolean';
    }

    protected function typeJson($column): string
    {
        return 'object';
    }

    protected function typeDate($column): string
    {
        return 'string';
    }

    protected function typeDateTime($column): string
    {
        return 'string';
    }

    protected function typeTimestamp($column): string
    {
        return 'string';
    }

    protected function typeUuid($column): string
    {
        return 'string';
    }

    protected function typeId($column): string
    {
        return 'string';
    }

    protected function typeMorphs($column): string
    {
        return 'string';
    }

    protected function typeNullableMorphs($column): string
    {
        return 'string';
    }

    public function wrap($value): string
    {
        if ($value instanceof \Illuminate\Database\Query\Expression) {
            return $value->getValue();
        }

        return '`' . $value . '`';
    }

    public function wrapTable($table): string
    {
        if ($table instanceof \Illuminate\Database\Query\Expression) {
            return $table->getValue();
        }

        return $this->wrap($table);
    }
}
