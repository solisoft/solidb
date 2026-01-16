<?php

namespace SoliDB;

class ScriptsClient
{
    private Client $client;

    public function __construct(Client $client)
    {
        $this->client = $client;
    }

    public function create(string $name, string $path, array $methods, string $code, ?string $description = null, ?string $collection = null): array
    {
        $params = [
            'database' => $this->client->getDatabase(),
            'name' => $name,
            'path' => $path,
            'methods' => $methods,
            'code' => $code,
        ];
        if ($description !== null) {
            $params['description'] = $description;
        }
        if ($collection !== null) {
            $params['collection'] = $collection;
        }
        $res = $this->client->sendCommand('create_script', $params);
        return $res['data'] ?? [];
    }

    public function list(): array
    {
        $res = $this->client->sendCommand('list_scripts', ['database' => $this->client->getDatabase()]);
        return $res['data'] ?? [];
    }

    public function get(string $scriptId): array
    {
        $res = $this->client->sendCommand('get_script', [
            'database' => $this->client->getDatabase(),
            'script_id' => $scriptId
        ]);
        return $res['data'] ?? [];
    }

    public function update(string $scriptId, array $updates): array
    {
        $res = $this->client->sendCommand('update_script', [
            'database' => $this->client->getDatabase(),
            'script_id' => $scriptId,
            'updates' => $updates
        ]);
        return $res['data'] ?? [];
    }

    public function delete(string $scriptId): void
    {
        $this->client->sendCommand('delete_script', [
            'database' => $this->client->getDatabase(),
            'script_id' => $scriptId
        ]);
    }

    public function getStats(): array
    {
        $res = $this->client->sendCommand('get_script_stats', []);
        return $res['data'] ?? [];
    }
}
