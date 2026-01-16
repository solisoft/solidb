<?php

namespace SoliDB;

class ApiKeysClient
{
    private Client $client;

    public function __construct(Client $client)
    {
        $this->client = $client;
    }

    public function list(): array
    {
        $res = $this->client->sendCommand('list_api_keys', []);
        return $res['data'] ?? [];
    }

    public function create(string $name, array $permissions, ?string $expiresAt = null): array
    {
        $args = ['name' => $name, 'permissions' => $permissions];
        if ($expiresAt !== null) {
            $args['expires_at'] = $expiresAt;
        }
        $res = $this->client->sendCommand('create_api_key', $args);
        return $res['data'] ?? [];
    }

    public function get(string $keyId): array
    {
        $res = $this->client->sendCommand('get_api_key', ['key_id' => $keyId]);
        return $res['data'] ?? [];
    }

    public function delete(string $keyId): void
    {
        $this->client->sendCommand('delete_api_key', ['key_id' => $keyId]);
    }

    public function regenerate(string $keyId): array
    {
        $res = $this->client->sendCommand('regenerate_api_key', ['key_id' => $keyId]);
        return $res['data'] ?? [];
    }
}
