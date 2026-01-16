<?php

namespace SoliDB;

use SoliDB\Exception\DriverException;

class Client
{
    private $socket; /* resource|false */
    private $host;
    private $port;
    private $isConnected = false;
    private ?string $database = null;

    // Magic header: "solidb-drv-v1" + null byte
    private const MAGIC_HEADER = "solidb-drv-v1\x00";

    // Default max message size (16 MB)
    private const MAX_MESSAGE_SIZE = 16777216;

    private $packer = null;
    private $unpacker = null;

    // Sub-clients
    private ?ScriptsClient $scriptsClient = null;
    private ?JobsClient $jobsClient = null;
    private ?CronClient $cronClient = null;
    private ?TriggersClient $triggersClient = null;
    private ?EnvClient $envClient = null;
    private ?RolesClient $rolesClient = null;
    private ?UsersClient $usersClient = null;
    private ?ApiKeysClient $apiKeysClient = null;
    private ?ClusterClient $clusterClient = null;
    private ?CollectionsOpsClient $collectionsOpsClient = null;
    private ?IndexesOpsClient $indexesOpsClient = null;
    private ?GeoClient $geoClient = null;
    private ?VectorClient $vectorClient = null;
    private ?TTLClient $ttlClient = null;
    private ?ColumnarClient $columnarClient = null;

    public function __construct(string $host = '127.0.0.1', int $port = 6745)
    {
        $this->host = $host;
        $this->port = $port;

        // Check for MessagePack support
        if (!function_exists('msgpack_pack')) {
            if (class_exists('MessagePack\Packer')) {
                $this->packer = new \MessagePack\Packer();
                $this->unpacker = new \MessagePack\BufferUnpacker();
            } else {
                throw new \RuntimeException("The 'msgpack' PHP extension or 'rybakit/msgpack' library is required.");
            }
        }
    }

    // Database context
    public function useDatabase(string $name): self
    {
        $this->database = $name;
        return $this;
    }

    public function getDatabase(): ?string
    {
        return $this->database;
    }

    // Sub-client accessors
    public function scripts(): ScriptsClient
    {
        if (!$this->scriptsClient) {
            $this->scriptsClient = new ScriptsClient($this);
        }
        return $this->scriptsClient;
    }

    public function jobs(): JobsClient
    {
        if (!$this->jobsClient) {
            $this->jobsClient = new JobsClient($this);
        }
        return $this->jobsClient;
    }

    public function cron(): CronClient
    {
        if (!$this->cronClient) {
            $this->cronClient = new CronClient($this);
        }
        return $this->cronClient;
    }

    public function triggers(): TriggersClient
    {
        if (!$this->triggersClient) {
            $this->triggersClient = new TriggersClient($this);
        }
        return $this->triggersClient;
    }

    public function env(): EnvClient
    {
        if (!$this->envClient) {
            $this->envClient = new EnvClient($this);
        }
        return $this->envClient;
    }

    public function roles(): RolesClient
    {
        if (!$this->rolesClient) {
            $this->rolesClient = new RolesClient($this);
        }
        return $this->rolesClient;
    }

    public function users(): UsersClient
    {
        if (!$this->usersClient) {
            $this->usersClient = new UsersClient($this);
        }
        return $this->usersClient;
    }

    public function apiKeys(): ApiKeysClient
    {
        if (!$this->apiKeysClient) {
            $this->apiKeysClient = new ApiKeysClient($this);
        }
        return $this->apiKeysClient;
    }

    public function cluster(): ClusterClient
    {
        if (!$this->clusterClient) {
            $this->clusterClient = new ClusterClient($this);
        }
        return $this->clusterClient;
    }

    public function collectionsOps(): CollectionsOpsClient
    {
        if (!$this->collectionsOpsClient) {
            $this->collectionsOpsClient = new CollectionsOpsClient($this);
        }
        return $this->collectionsOpsClient;
    }

    public function indexesOps(): IndexesOpsClient
    {
        if (!$this->indexesOpsClient) {
            $this->indexesOpsClient = new IndexesOpsClient($this);
        }
        return $this->indexesOpsClient;
    }

    public function geo(): GeoClient
    {
        if (!$this->geoClient) {
            $this->geoClient = new GeoClient($this);
        }
        return $this->geoClient;
    }

    public function vector(): VectorClient
    {
        if (!$this->vectorClient) {
            $this->vectorClient = new VectorClient($this);
        }
        return $this->vectorClient;
    }

    public function ttl(): TTLClient
    {
        if (!$this->ttlClient) {
            $this->ttlClient = new TTLClient($this);
        }
        return $this->ttlClient;
    }

    public function columnar(): ColumnarClient
    {
        if (!$this->columnarClient) {
            $this->columnarClient = new ColumnarClient($this);
        }
        return $this->columnarClient;
    }

    public function connect(): void
    {
        if ($this->isConnected) {
            return;
        }

        $address = "tcp://{$this->host}:{$this->port}";
        $this->socket = @stream_socket_client(
            $address,
            $errno,
            $errstr,
            5, // 5 seconds timeout
            STREAM_CLIENT_CONNECT
        );

        if (!$this->socket) {
            throw new DriverException("Failed to connect to SoliDB at $address: $errstr", "connection_error");
        }

        // Set read timeout
        stream_set_timeout($this->socket, 30);

        // Turn off buffering
        stream_set_write_buffer($this->socket, 0);

        // Send handshake
        $sent = fwrite($this->socket, self::MAGIC_HEADER);
        if ($sent !== strlen(self::MAGIC_HEADER)) {
            fclose($this->socket);
            throw new DriverException("Failed to send handshake", "connection_error");
        }

        $this->isConnected = true;
    }

    public function close(): void
    {
        if ($this->socket) {
            fclose($this->socket);
            $this->socket = null;
        }
        $this->isConnected = false;
    }

    private function pack($data)
    {
        if ($this->packer) {
            return $this->packer->pack($data);
        }
        return msgpack_pack($data);
    }

    private function unpack($data)
    {
        if ($this->unpacker) {
            $this->unpacker->reset($data);
            return $this->unpacker->unpack();
        }
        return msgpack_unpack($data);
    }

    // Public for sub-clients to use
    public function sendCommand(string $cmdName, array $params = []): array
    {
        if (!$this->isConnected) {
            $this->connect();
        }

        // Construct command payload
        $command = array_merge(['cmd' => $cmdName], $params);

        // Pack payload
        try {
            $payload = $this->pack($command);
        } catch (\Exception $e) {
            throw new DriverException("Failed to serialize command: " . $e->getMessage(), "serialization_error");
        }

        $length = strlen($payload);

        // Pack length (4 bytes, big-endian)
        $header = pack('N', $length);

        // Send
        $data = $header . $payload;
        $totalSent = 0;
        $totalLen = strlen($data);

        while ($totalSent < $totalLen) {
            $sent = fwrite($this->socket, substr($data, $totalSent));
            if ($sent === false) {
                $this->isConnected = false;
                throw new DriverException("Failed to send data to server", "connection_error");
            }
            $totalSent += $sent;
        }

        return $this->receive();
    }

    private function receive(): array
    {
        // Read 4 bytes length
        $header = $this->readBytes(4);
        if ($header === false) {
            $this->isConnected = false;
            throw new DriverException("Server closed connection during receive", "connection_error");
        }

        $length = unpack('N', $header)[1];

        if ($length > self::MAX_MESSAGE_SIZE) {
            throw new DriverException("Response too large: $length bytes", "protocol_error");
        }

        // Read payload
        $payload = $this->readBytes($length);
        if ($payload === false) {
            $this->isConnected = false;
            throw new DriverException("Failed to read response payload", "connection_error");
        }

        // Unpack
        try {
            $response = $this->unpack($payload);
        } catch (\Exception $e) {
            throw new DriverException("Failed to deserialize response: " . $e->getMessage(), "serialization_error");
        }

        if (isset($response['status'])) {
            // Already a map (e.g. JSON-like serialization)
        } elseif (is_array($response) && isset($response[0]) && is_string($response[0])) {
            // Tuple format: [status, body]
            $status = $response[0];
            $body = $response[1] ?? null;

            $response = ['status' => $status];
            if ($status === 'ok') {
                $response['data'] = $body;
                // Handle other fields if packed in body tuple?
                // Assuming body IS data for now based on observation.
            } elseif ($status === 'error') {
                $response['error'] = $body;
            } elseif ($status === 'pong') {
                // Pong body is timestamp?
                // If pong { timestamp }, body might be map or value.
            }
        }

        // Check for error response
        if (isset($response['status']) && $response['status'] === 'error') {
            $err = $response['error'] ?? 'Unknown error';
            throw new DriverException("SoliDB Error: " . json_encode($err), "server_error");
        }

        return $response;
    }

    private function readBytes(int $len)
    {
        $buffer = '';
        $remaining = $len;

        while ($remaining > 0) {
            $chunk = fread($this->socket, $remaining);
            if ($chunk === false || $chunk === '') {
                return false;
            }
            $buffer .= $chunk;
            $remaining -= strlen($chunk);
        }

        return $buffer;
    }

    // =========================================================================
    // Public API
    // =========================================================================

    public function ping(): float
    {
        $start = microtime(true);
        $this->sendCommand('ping');
        return (microtime(true) - $start) * 1000;
    }

    public function auth(string $database, string $username, string $password): void
    {
        $this->sendCommand('auth', [
            'database' => $database,
            'username' => $username,
            'password' => $password
        ]);
    }

    // --- Database Operations ---

    public function listDatabases(): array
    {
        $res = $this->sendCommand('list_databases');
        return $res['data'] ?? [];
    }

    public function createDatabase(string $name): void
    {
        $this->sendCommand('create_database', ['name' => $name]);
    }

    public function deleteDatabase(string $name): void
    {
        $this->sendCommand('delete_database', ['name' => $name]);
    }

    // --- Collection Operations ---

    public function listCollections(string $database): array
    {
        $res = $this->sendCommand('list_collections', ['database' => $database]);
        return $res['data'] ?? [];
    }

    public function createCollection(string $database, string $name, ?string $type = null): void
    {
        $args = ['database' => $database, 'name' => $name];
        if ($type) {
            $args['type'] = $type;
        }
        $this->sendCommand('create_collection', $args);
    }

    public function deleteCollection(string $database, string $name): void
    {
        $this->sendCommand('delete_collection', ['database' => $database, 'name' => $name]);
    }

    public function collectionStats(string $database, string $name): array
    {
        $res = $this->sendCommand('collection_stats', ['database' => $database, 'name' => $name]);
        return $res['data'] ?? [];
    }

    // --- Document Operations ---

    public function insert(string $database, string $collection, array $document, ?string $key = null): array
    {
        $res = $this->sendCommand('insert', [
            'database' => $database,
            'collection' => $collection,
            'document' => $document,
            'key' => $key
        ]);
        if (!array_key_exists('data', $res)) {
            throw new DriverException("Insert response missing data. Keys: " . implode(',', array_keys($res)) . ". Response: " . json_encode($res));
        }
        return $res['data'];
    }

    public function get(string $database, string $collection, string $key): ?array
    {
        try {
            $res = $this->sendCommand('get', [
                'database' => $database,
                'collection' => $collection,
                'key' => $key
            ]);
            return $res['data'] ?? null;
        } catch (DriverException $e) {
            // Need to verify if 404 is an error or null return in protocol
            // Rust client usually returns Error if not found
            throw $e;
        }
    }

    public function update(string $database, string $collection, string $key, array $document, bool $merge = true): void
    {
        $this->sendCommand('update', [
            'database' => $database,
            'collection' => $collection,
            'key' => $key,
            'document' => $document,
            'merge' => $merge
        ]);
    }

    public function delete(string $database, string $collection, string $key): void
    {
        $this->sendCommand('delete', [
            'database' => $database,
            'collection' => $collection,
            'key' => $key
        ]);
    }

    // --- Query Operations ---

    public function query(string $database, string $sdbql, array $bindVars = []): array
    {
        $res = $this->sendCommand('query', [
            'database' => $database,
            'sdbql' => $sdbql,
            'bind_vars' => (object) $bindVars // Ensure map
        ]);
        return $res['data'] ?? [];
    }

    public function explain(string $database, string $sdbql, array $bindVars = []): array
    {
        $res = $this->sendCommand('explain', [
            'database' => $database,
            'sdbql' => $sdbql,
            'bind_vars' => (object) $bindVars
        ]);
        return $res['data'] ?? [];
    }

    // --- Index Operations ---

    public function createIndex(string $database, string $collection, string $name, array $fields, bool $unique = false, bool $sparse = false): void
    {
        $this->sendCommand('create_index', [
            'database' => $database,
            'collection' => $collection,
            'name' => $name,
            'fields' => $fields,
            'unique' => $unique,
            'sparse' => $sparse
        ]);
    }

    public function listIndexes(string $database, string $collection): array
    {
        $res = $this->sendCommand('list_indexes', [
            'database' => $database,
            'collection' => $collection
        ]);
        return $res['data'] ?? [];
    }

    public function deleteIndex(string $database, string $collection, string $name): void
    {
        $this->sendCommand('delete_index', [
            'database' => $database,
            'collection' => $collection,
            'name' => $name
        ]);
    }

    // --- Transactions ---

    public function beginTransaction(string $database, string $isolationLevel = 'read_committed'): string
    {
        $res = $this->sendCommand('begin_transaction', [
            'database' => $database,
            'isolation_level' => $isolationLevel
        ]);
        return $res['tx_id'];
    }

    public function commitTransaction(string $txId): void
    {
        $this->sendCommand('commit_transaction', ['tx_id' => $txId]);
    }

    public function rollbackTransaction(string $txId): void
    {
        $this->sendCommand('rollback_transaction', ['tx_id' => $txId]);
    }
}
