<?php

namespace SoliDB\Laravel;

use Illuminate\Database\Connection;
use Illuminate\Database\Connectors\ConnectionFactory;
use Illuminate\Support\ServiceProvider;
use SoliDB\Laravel\Connections\SoliDBConnection;
use SoliDB\Laravel\Connectors\SoliDBConnector;
use SoliDB\Laravel\Query\Builders\SoliDBBuilder;
use SoliDB\Laravel\Query\Grammars\Grammar as QueryGrammar;
use SoliDB\Laravel\Query\Processors\SoliDBProcessor;
use SoliDB\Laravel\Schema\Builders\SoliDBSchemaBuilder;
use SoliDB\Laravel\Schema\Grammars\Grammar as SchemaGrammar;

class SoliDBServiceProvider extends ServiceProvider
{
    public function register(): void
    {
        $this->app->resolving(ConnectionFactory::class, function ($factory) {
            $factory->extend('solidb', function ($config, $name) {
                $config['name'] = $name;
                
                $connection = new SoliDBConnection($config);
                
                $connection->setQueryGrammar(new QueryGrammar());
                $connection->setSchemaGrammar(new SchemaGrammar());
                $connection->setPostProcessor(new SoliDBProcessor());
                
                return $connection;
            });
        });
    }

    public function boot(): void
    {
        $this->publishes([
            __DIR__ . '/../config/solidb.php' => config_path('solidb.php'),
        ], 'solidb-config');
    }
}
