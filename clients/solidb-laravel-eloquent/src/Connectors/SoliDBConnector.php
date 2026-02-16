<?php

namespace SoliDB\Laravel\Connectors;

use Illuminate\Database\Connectors\Connector;
use Illuminate\Database\Connectors\ConnectorInterface;
use SoliDB\Client as SoliDBClient;

class SoliDBConnector extends Connector implements ConnectorInterface
{
    public function connect(array $config)
    {
        $client = new SoliDBClient(
            $config['host'] ?? '127.0.0.1',
            $config['port'] ?? 6745
        );

        if (!empty($config['database'])) {
            $client->useDatabase($config['database']);
        }

        if (!empty($config['username']) && !empty($config['password'])) {
            $client->auth(
                $config['database'] ?? 'default',
                $config['username'],
                $config['password']
            );
        } elseif (!empty($config['api_key'])) {
            $client->authWithApiKey(
                $config['database'] ?? 'default',
                $config['api_key']
            );
        }

        return $client;
    }
}
