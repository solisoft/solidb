<?php

require_once __DIR__ . '/../vendor/autoload.php';

use SoliDB\Client;

function assert_true($condition, $message = 'Assertion failed')
{
    if (!$condition) {
        throw new Exception($message);
    }
}

function test($name, $closure)
{
    echo "Running: $name... ";
    try {
        $closure();
        echo "OK\n";
    } catch (Exception $e) {
        echo "FAILED: " . $e->getMessage() . "\n";
        exit(1);
    }
}

$port = getenv('SOLIDB_PORT') ? (int) getenv('SOLIDB_PORT') : 6745;
$client = new Client('127.0.0.1', $port);
$authFunc = function () use ($client) {
    // Try auth, ignore if fails (might be already authed or not needed)
    try {
        $client->auth('_system', 'admin', 'admin');
    } catch (Exception $e) {
    }
};
$authFunc();

echo "Running Simple Tests (Fallback)...\n";

test('connection', function () use ($client) {
    $client->ping();
});

test('crud', function () use ($client) {
    $db = 'simple_test_db';
    try {
        $client->deleteDatabase($db);
    } catch (Exception $e) {
        $msg = $e->getMessage();
        // If error contains "not found", ignore. Otherwise rethrow or print
        if (strpos($msg, 'not found') === false) {
            echo "Warning: deleteDatabase failed: $msg\n";
        }
    }
    try {
        $client->createDatabase($db);
    } catch (Exception $e) { /* Ignore if exists, but we expect it not to exist */
    }

    try {
        $client->createCollection($db, 'users');
    } catch (Exception $e) {
        // If collection already exists, ignore
        if (strpos($e->getMessage(), 'already exists') === false) {
            throw $e;
        }
    }

    $doc = $client->insert($db, 'users', ['name' => 'Simple', 'val' => 1]);
    if (!isset($doc['_key'])) {
        var_dump($doc);
        throw new Exception('Insert failed: no _key');
    }

    $fetched = $client->get($db, 'users', $doc['_key']);
    assert_true($fetched['name'] === 'Simple', 'Get failed');

    $client->deleteDatabase($db);
});

echo "All simple tests passed.\n";
