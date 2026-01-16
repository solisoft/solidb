<?php

namespace SoliDB;

class JobsClient
{
    private Client $client;

    public function __construct(Client $client)
    {
        $this->client = $client;
    }

    public function listQueues(): array
    {
        $res = $this->client->sendCommand('list_queues', ['database' => $this->client->getDatabase()]);
        return $res['data'] ?? [];
    }

    public function listJobs(string $queueName, ?string $status = null, ?int $limit = null, ?int $offset = null): array
    {
        $params = [
            'database' => $this->client->getDatabase(),
            'queue_name' => $queueName
        ];
        if ($status !== null) {
            $params['status'] = $status;
        }
        if ($limit !== null) {
            $params['limit'] = $limit;
        }
        if ($offset !== null) {
            $params['offset'] = $offset;
        }
        $res = $this->client->sendCommand('list_jobs', $params);
        return $res['data'] ?? [];
    }

    public function enqueue(string $queueName, string $scriptPath, ?array $params = null, ?int $priority = null, ?string $runAt = null): array
    {
        $args = [
            'database' => $this->client->getDatabase(),
            'queue_name' => $queueName,
            'script_path' => $scriptPath
        ];
        if ($params !== null) {
            $args['params'] = $params;
        }
        if ($priority !== null) {
            $args['priority'] = $priority;
        }
        if ($runAt !== null) {
            $args['run_at'] = $runAt;
        }
        $res = $this->client->sendCommand('enqueue_job', $args);
        return $res['data'] ?? [];
    }

    public function cancel(string $jobId): void
    {
        $this->client->sendCommand('cancel_job', [
            'database' => $this->client->getDatabase(),
            'job_id' => $jobId
        ]);
    }

    public function get(string $jobId): array
    {
        $res = $this->client->sendCommand('get_job', [
            'database' => $this->client->getDatabase(),
            'job_id' => $jobId
        ]);
        return $res['data'] ?? [];
    }
}
