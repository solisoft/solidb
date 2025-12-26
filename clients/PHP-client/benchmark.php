<?php
require_once __DIR__ . '/vendor/autoload.php';
require_once __DIR__ . '/src/Client.php';
require_once __DIR__ . '/src/Exception/DriverException.php';

use SoliDB\Client;

function run_benchmark()
{
    $port = intval(getenv('SOLIDB_PORT') ?: '9998');
    $password = getenv('SOLIDB_PASSWORD') ?: 'password';

    $client = new Client('127.0.0.1', $port);
    $client->auth('_system', 'admin', $password);

    $db = 'bench_db';
    $col = 'php_bench';

    try {
        $client->createDatabase($db);
    } catch (\Exception $e) {
    }

    try {
        $client->createCollection($db, $col);
    } catch (\Exception $e) {
    }

    $iterations = 1000;

    // INSERT BENCHMARK
    $insertedKeys = [];
    $start_time = microtime(true);
    for ($i = 0; $i < $iterations; $i++) {
        $result = $client->insert($db, $col, ['id' => $i, 'data' => 'benchmark data content']);
        if (isset($result['_key'])) {
            $insertedKeys[] = $result['_key'];
        }
    }
    $end_time = microtime(true);

    $insert_duration = $end_time - $start_time;
    $insert_ops_per_sec = $iterations / $insert_duration;
    echo "PHP_BENCH_RESULT:" . round($insert_ops_per_sec, 2) . "\n";

    // READ BENCHMARK
    if (count($insertedKeys) > 0) {
        $start_time = microtime(true);
        for ($i = 0; $i < $iterations; $i++) {
            $key = $insertedKeys[$i % count($insertedKeys)];
            $client->get($db, $col, $key);
        }
        $end_time = microtime(true);

        $read_duration = $end_time - $start_time;
        $read_ops_per_sec = $iterations / $read_duration;
        echo "PHP_READ_BENCH_RESULT:" . round($read_ops_per_sec, 2) . "\n";
    }
}

run_benchmark();
