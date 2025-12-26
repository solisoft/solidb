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

    $start_time = microtime(true);
    for ($i = 0; $i < $iterations; $i++) {
        $client->insert($db, $col, ['id' => $i, 'data' => 'benchmark data content']);
    }
    $end_time = microtime(true);

    $duration = $end_time - $start_time;
    $ops_per_sec = $iterations / $duration;

    echo "PHP_BENCH_RESULT:" . round($ops_per_sec, 2) . "\n";
}

run_benchmark();
