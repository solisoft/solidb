<?php

namespace SoliDB;

class TriggersClient
{
    private Client $client;

    public function __construct(Client $client)
    {
        $this->client = $client;
    }

    public function list(): array
    {
        $res = $this->client->sendCommand('list_triggers', ['database' => $this->client->getDatabase()]);
        return $res['data'] ?? [];
    }

    public function listByCollection(string $collection): array
    {
        $res = $this->client->sendCommand('list_triggers_by_collection', [
            'database' => $this->client->getDatabase(),
            'collection' => $collection
        ]);
        return $res['data'] ?? [];
    }

    public function create(string $name, string $collection, string $event, string $timing, string $scriptPath, ?bool $enabled = null): array
    {
        $args = [
            'database' => $this->client->getDatabase(),
            'name' => $name,
            'collection' => $collection,
            'event' => $event,
            'timing' => $timing,
            'script_path' => $scriptPath
        ];
        if ($enabled !== null) {
            $args['enabled'] = $enabled;
        }
        $res = $this->client->sendCommand('create_trigger', $args);
        return $res['data'] ?? [];
    }

    public function get(string $triggerId): array
    {
        $res = $this->client->sendCommand('get_trigger', [
            'database' => $this->client->getDatabase(),
            'trigger_id' => $triggerId
        ]);
        return $res['data'] ?? [];
    }

    public function update(string $triggerId, array $updates): array
    {
        $res = $this->client->sendCommand('update_trigger', [
            'database' => $this->client->getDatabase(),
            'trigger_id' => $triggerId,
            'updates' => $updates
        ]);
        return $res['data'] ?? [];
    }

    public function delete(string $triggerId): void
    {
        $this->client->sendCommand('delete_trigger', [
            'database' => $this->client->getDatabase(),
            'trigger_id' => $triggerId
        ]);
    }

    public function toggle(string $triggerId, bool $enabled): void
    {
        $this->client->sendCommand('toggle_trigger', [
            'database' => $this->client->getDatabase(),
            'trigger_id' => $triggerId,
            'enabled' => $enabled
        ]);
    }
}
