<?php

namespace SoliDB;

class EnvClient
{
    private Client $client;

    public function __construct(Client $client)
    {
        $this->client = $client;
    }

    public function list(): array
    {
        $res = $this->client->sendCommand('list_env_vars', ['database' => $this->client->getDatabase()]);
        return $res['data'] ?? [];
    }

    public function get(string $key): mixed
    {
        $res = $this->client->sendCommand('get_env_var', [
            'database' => $this->client->getDatabase(),
            'key' => $key
        ]);
        return $res['data'] ?? null;
    }

    public function set(string $key, mixed $value): void
    {
        $this->client->sendCommand('set_env_var', [
            'database' => $this->client->getDatabase(),
            'key' => $key,
            'value' => $value
        ]);
    }

    public function delete(string $key): void
    {
        $this->client->sendCommand('delete_env_var', [
            'database' => $this->client->getDatabase(),
            'key' => $key
        ]);
    }

    public function setBulk(array $vars): void
    {
        $this->client->sendCommand('set_env_vars_bulk', [
            'database' => $this->client->getDatabase(),
            'vars' => $vars
        ]);
    }
}
