<?php

namespace SoliDB\Laravel\Query\Processors;

use Illuminate\Database\Query\Processors\Processor as BaseProcessor;

class SoliDBProcessor extends BaseProcessor
{
    public function processSelect($query, $results)
    {
        return $results;
    }

    public function processInsert($query, $results)
    {
        return $results;
    }

    public function processUpdate($query, $results)
    {
        return $results['updated'] ?? 0;
    }

    public function processDelete($query, $results)
    {
        return $results['deleted'] ?? 0;
    }
}
