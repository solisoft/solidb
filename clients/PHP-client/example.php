<?php

require_once __DIR__ . '/vendor/autoload.php';

// If running without composer/autoloader for testing:
if (!class_exists('SoliDB\Client')) {
    require_once __DIR__ . '/src/Exception/DriverException.php';
    require_once __DIR__ . '/src/Client.php';
}

use SoliDB\Client;
use SoliDB\Exception\DriverException;

// Note: Ensure 'msgpack' extension is installed.
// php -d extension=msgpack.so example.php

try {
    echo "Connecting to SoliDB...\n";
    $client = new Client('127.0.0.1', 6745);
    $client->connect();

    // Ping
    $latency = $client->ping();
    echo "Ping: {$latency}ms\n";

    // Setup
    $dbName = 'php_test_db';

    // Check if db exists and delete (cleanup)
    $dbs = $client->listDatabases();
    foreach ($dbs as $db) {
        if ($db['name'] === $dbName) {
            echo "Deleting existing database...\n";
            $client->deleteDatabase($dbName);
            break;
        }
    }

    // Create DB
    echo "Creating database '$dbName'...\n";
    $client->createDatabase($dbName);

    // Auth (if needed - creating db automatically auths usually, but let's test)
    // $client->auth($dbName, 'admin', 'password');

    // Create Collection
    echo "Creating collection 'users'...\n";
    $client->createCollection($dbName, 'users');

    // Insert Document
    echo "Inserting document...\n";
    $doc = [
        'name' => 'John Doe',
        'email' => 'john@example.com',
        'age' => 30,
        'tags' => ['developer', 'php']
    ];
    $inserted = $client->insert($dbName, 'users', $doc);
    echo "Inserted: " . json_encode($inserted) . "\n";

    // Get Document (assuming insert returns the doc or we query it)
    $key = $inserted['_key'] ?? null;
    if ($key) {
        echo "Retrieving document $key...\n";
        $fetched = $client->get($dbName, 'users', $key);
        echo "Fetched: " . json_encode($fetched) . "\n";
    }

    // Query
    echo "Querying users...\n";
    $results = $client->query($dbName, "FOR u IN users FILTER u.age >= @min_age RETURN u", ['min_age' => 25]);
    echo "Query Results: " . count($results) . " found.\n";
    print_r($results);

    // Transaction
    echo "Starting transaction...\n";
    $txId = $client->beginTransaction($dbName);
    echo "Tx ID: $txId\n";

    $client->insert($dbName, 'users', ['name' => 'Jane Doe', 'age' => 28]);

    echo "Committing transaction...\n";
    $client->commitTransaction($txId);

    // Verify transaction
    $count = $client->query($dbName, "RETURN COUNT(FOR u IN users RETURN 1)");
    echo "Total users: " . json_encode($count) . "\n";

    echo "Done.\n";

} catch (DriverException $e) {
    echo "Driver Error: " . $e->getMessage() . " (Type: " . $e->getErrorType() . ")\n";
    exit(1);
} catch (\Exception $e) {
    echo "Error: " . $e->getMessage() . "\n";
    exit(1);
}
