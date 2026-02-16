<?php

namespace SoliDB\Laravel\Facades;

use Illuminate\Support\Facades\Facade;

class SoliDB extends Facade
{
    protected static function getFacadeAccessor(): string
    {
        return 'db';
    }
}
