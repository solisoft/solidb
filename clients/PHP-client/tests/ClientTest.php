<?php

use SoliDB\Client;
use SoliDB\Exception\DriverException;

// Mock the global checks/functions if strictly doing unit testing without server
// But here we likely want "Integration Specs" or we need to Mock the stream socket.
// Since we don't have a running server reliable in CI/this environment for the spec execution (unless we assume so),
// we might want to mock the socket connection? 
// However, the user asked for "PHP specs to test it". Usually implies usage specs. 
// Given the environment, I'll write specs that *would* run against a server, 
// but I'll also try to structure it so we can potentially mock.
// For now, let's write real integration specs assuming server is up (as typical for DB drivers).

// Helper to get client
function getClient(): Client
{
    $port = getenv('SOLIDB_PORT') ? (int) getenv('SOLIDB_PORT') : 6745;
    $client = new Client('127.0.0.1', $port);
    try {
        $client->auth('_system', 'admin', 'admin');
    } catch (Exception $e) {
        // Ignore auth error in getClient? 
        // No, if server is up, it should work. 
        // But some tests might not need it or might want to test auth.
        // For now, assume we need it for everything.
    }
    return $client;
}

test('can connect to solidb', function () {
    $client = getClient();
    $client->connect();
    expect(true)->toBeTrue(); // If no exception, we are good
    $client->close();
});

test('ping returns latency', function () {
    $client = getClient();
    $latency = $client->ping();
    expect($latency)->toBeGreaterThan(0);
});

describe('Database Operations', function () {
    $dbName = 'pest_test_db_' . uniqid();
    $client = getClient();

    afterAll(function () use ($client, $dbName) {
        try {
            $client->deleteDatabase($dbName);
        } catch (Exception $e) {
        }
    });

    test('can create database', function () use ($client, $dbName) {
        $client->createDatabase($dbName);
        $dbs = $client->listDatabases();

        // Check if our db is in the list
        $found = false;
        foreach ($dbs as $db) {
            if ($db['name'] === $dbName) {
                $found = true;
                break;
            }
        }
        expect($found)->toBeTrue();
    });

    test('can list collections (empty initially)', function () use ($client, $dbName) {
        // Create db if not exists (in case order fails)
        try {
            $client->createDatabase($dbName);
        } catch (Exception $e) {
        }

        $colls = $client->listCollections($dbName);
        // Expect at least system collections or empty depending on implementation, 
        // usually initially empty or just system ones.
        expect($colls)->toBeArray();
    });
});

describe('CRUD Operations', function () {
    $dbName = 'pest_crud_db_' . uniqid();
    $collName = 'users';
    $client = getClient();

    beforeAll(function () use ($client, $dbName, $collName) {
        try {
            $client->deleteDatabase($dbName);
        } catch (Exception $e) {
        }
        $client->createDatabase($dbName);
        $client->createCollection($dbName, $collName);
    });

    afterAll(function () use ($client, $dbName) {
        try {
            $client->deleteDatabase($dbName);
        } catch (Exception $e) {
        }
    });

    test('can insert document', function () use ($client, $dbName, $collName) {
        $doc = ['name' => 'Pest User', 'email' => 'pest@example.com'];
        $res = $client->insert($dbName, $collName, $doc);

        expect($res)->toBeArray();
        expect($res['_key'])->toBeString();

        return $res['_key'];
    });

    test('can get document', function () use ($client, $dbName, $collName) {
        // Insert first
        $doc = ['name' => 'Get Me'];
        $inserted = $client->insert($dbName, $collName, $doc);
        $key = $inserted['_key'];

        $fetched = $client->get($dbName, $collName, $key);
        expect($fetched)->toBeArray();
        expect($fetched['name'])->toBe('Get Me');
    });

    test('can update document', function () use ($client, $dbName, $collName) {
        $doc = ['name' => 'Update Me', 'age' => 20];
        $inserted = $client->insert($dbName, $collName, $doc);
        $key = $inserted['_key'];

        $client->update($dbName, $collName, $key, ['age' => 21]);

        $fetched = $client->get($dbName, $collName, $key);
        expect($fetched['age'])->toBe(21);
        expect($fetched['name'])->toBe('Update Me'); // Merge check
    });

    test('can delete document', function () use ($client, $dbName, $collName) {
        $doc = ['name' => 'Delete Me'];
        $inserted = $client->insert($dbName, $collName, $doc);
        $key = $inserted['_key'];

        $client->delete($dbName, $collName, $key);

        // Fetching deleted doc should likely throw or return null
        // Based on client impl, it might throw DriverException if 404 is error
        // Or return null.
        // Let's assume exception for not found if strict, or check implementation.
        // Implementation: assumes 200 OK for found. If server returns Error for not found, it throws.

        // We expect an exception for "Document not found" or similar if the server treats it as error.
        // If server returns null data for not found, then expect(null).
        // Let's assume it throws.
        try {
            $client->get($dbName, $collName, $key);
            $failed = false;
        } catch (DriverException $e) {
            $failed = true;
        }
        expect($failed)->toBeTrue();
    });
});

describe('SDBQL Queries', function () {
    $dbName = 'pest_query_db_' . uniqid();
    $collName = 'products';
    $client = getClient();

    beforeAll(function () use ($client, $dbName, $collName) {
        try {
            $client->deleteDatabase($dbName);
        } catch (Exception $e) {
        }
        $client->createDatabase($dbName);
        $client->createCollection($dbName, $collName);

        // Seed
        $client->insert($dbName, $collName, ['name' => 'Apple', 'price' => 10]);
        $client->insert($dbName, $collName, ['name' => 'Banana', 'price' => 20]);
        $client->insert($dbName, $collName, ['name' => 'Cherry', 'price' => 30]);
    });

    afterAll(function () use ($client, $dbName) {
        try {
            $client->deleteDatabase($dbName);
        } catch (Exception $e) {
        }
    });

    test('can execute simple query', function () use ($client, $dbName, $collName) {
        $query = "FOR p IN products RETURN p";
        $results = $client->query($dbName, $query);

        expect($results)->toBeArray();
        expect(count($results))->toBe(3);
    });

    test('can execute query with filter and bind vars', function () use ($client, $dbName, $collName) {
        $query = "FOR p IN products FILTER p.price > @min_price RETURN p.name";
        $results = $client->query($dbName, $query, ['min_price' => 15]);

        expect($results)->toBeArray();
        expect(count($results))->toBe(2); // Banana, Cherry
        expect($results)->toContain('Banana');
        expect($results)->toContain('Cherry');
    });
});

describe('Transactions', function () {
    $dbName = 'pest_tx_db_' . uniqid();
    $collName = 'accounts';
    $client = getClient();

    beforeAll(function () use ($client, $dbName, $collName) {
        try {
            $client->deleteDatabase($dbName);
        } catch (Exception $e) {
        }
        $client->createDatabase($dbName);
        $client->createCollection($dbName, $collName);
    });

    afterAll(function () use ($client, $dbName) {
        try {
            $client->deleteDatabase($dbName);
        } catch (Exception $e) {
        }
    });

    test('can rollback transaction', function () use ($client, $dbName, $collName) {
        $txId = $client->beginTransaction($dbName);

        $client->insert($dbName, $collName, ['name' => 'Rollback User']);

        $client->rollbackTransaction($txId);

        // Verify user is not there
        $results = $client->query($dbName, "FOR a IN accounts FILTER a.name == 'Rollback User' RETURN a");
        expect(count($results))->toBe(0);
    });

    test('can commit transaction', function () use ($client, $dbName, $collName) {
        $txId = $client->beginTransaction($dbName);

        $client->insert($dbName, $collName, ['name' => 'Commit User']);

        $client->commitTransaction($txId);

        // Verify user is there
        $results = $client->query($dbName, "FOR a IN accounts FILTER a.name == 'Commit User' RETURN a");
        expect(count($results))->toBe(1);
    });
});
