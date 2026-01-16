<?php

namespace SoliDB;

class RolesClient
{
    private Client $client;

    public function __construct(Client $client)
    {
        $this->client = $client;
    }

    public function list(): array
    {
        $res = $this->client->sendCommand('list_roles', []);
        return $res['data'] ?? [];
    }

    public function create(string $name, array $permissions, ?string $description = null): array
    {
        $args = ['name' => $name, 'permissions' => $permissions];
        if ($description !== null) {
            $args['description'] = $description;
        }
        $res = $this->client->sendCommand('create_role', $args);
        return $res['data'] ?? [];
    }

    public function get(string $name): array
    {
        $res = $this->client->sendCommand('get_role', ['role_name' => $name]);
        return $res['data'] ?? [];
    }

    public function update(string $name, array $permissions, ?string $description = null): array
    {
        $args = ['role_name' => $name, 'permissions' => $permissions];
        if ($description !== null) {
            $args['description'] = $description;
        }
        $res = $this->client->sendCommand('update_role', $args);
        return $res['data'] ?? [];
    }

    public function delete(string $name): void
    {
        $this->client->sendCommand('delete_role', ['role_name' => $name]);
    }
}
