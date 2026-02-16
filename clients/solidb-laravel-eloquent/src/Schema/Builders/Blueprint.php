<?php

namespace SoliDB\Laravel\Schema\Builders;

use Illuminate\Database\Schema\Blueprint as BaseBlueprint;

class Blueprint extends BaseBlueprint
{
    public function create(): void
    {
        $this->connection->statement(
            $this->grammar->compileCreate($this)
        );
    }

    public function drop(): void
    {
        $this->connection->statement(
            $this->grammar->compileDrop($this)
        );
    }
}
