<?php

namespace SoliDB;

class CronClient
{
    private Client $client;

    public function __construct(Client $client)
    {
        $this->client = $client;
    }

    public function list(): array
    {
        $res = $this->client->sendCommand('list_cron_jobs', ['database' => $this->client->getDatabase()]);
        return $res['data'] ?? [];
    }

    public function create(string $name, string $schedule, string $scriptPath, ?array $params = null, ?bool $enabled = null, ?string $description = null): array
    {
        $args = [
            'database' => $this->client->getDatabase(),
            'name' => $name,
            'schedule' => $schedule,
            'script_path' => $scriptPath
        ];
        if ($params !== null) {
            $args['params'] = $params;
        }
        if ($enabled !== null) {
            $args['enabled'] = $enabled;
        }
        if ($description !== null) {
            $args['description'] = $description;
        }
        $res = $this->client->sendCommand('create_cron_job', $args);
        return $res['data'] ?? [];
    }

    public function get(string $cronId): array
    {
        $res = $this->client->sendCommand('get_cron_job', [
            'database' => $this->client->getDatabase(),
            'cron_id' => $cronId
        ]);
        return $res['data'] ?? [];
    }

    public function update(string $cronId, array $updates): array
    {
        $res = $this->client->sendCommand('update_cron_job', [
            'database' => $this->client->getDatabase(),
            'cron_id' => $cronId,
            'updates' => $updates
        ]);
        return $res['data'] ?? [];
    }

    public function delete(string $cronId): void
    {
        $this->client->sendCommand('delete_cron_job', [
            'database' => $this->client->getDatabase(),
            'cron_id' => $cronId
        ]);
    }

    public function toggle(string $cronId, bool $enabled): void
    {
        $this->client->sendCommand('toggle_cron_job', [
            'database' => $this->client->getDatabase(),
            'cron_id' => $cronId,
            'enabled' => $enabled
        ]);
    }
}
