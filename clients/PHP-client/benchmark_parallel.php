<?php
require_once __DIR__ . '/vendor/autoload.php';
require_once __DIR__ . '/src/Client.php';
require_once __DIR__ . '/src/Exception/DriverException.php';

use SoliDB\Client;

function run_parallel_benchmark()
{
    $port = intval(getenv('SOLIDB_PORT') ?: '9998');
    $password = getenv('SOLIDB_PASSWORD') ?: 'password';

    $numWorkers = 16;
    $totalInserts = 10000;
    $insertsPerWorker = $totalInserts / $numWorkers;

    $db = 'bench_db';
    $col = 'php_parallel_bench';

    // Setup: create database and collection
    $setupClient = new Client('127.0.0.1', $port);
    $setupClient->auth('_system', 'admin', $password);

    try {
        $setupClient->createDatabase($db);
    } catch (\Exception $e) {
    }

    try {
        $setupClient->createCollection($db, $col);
    } catch (\Exception $e) {
    }

    $startTime = microtime(true);

    // PHP doesn't have true threading, but we can use pcntl_fork for processes
    // or parallel extension. For simplicity, we'll use sequential with batching
    // as a workaround since parallel is not always available

    if (function_exists('pcntl_fork')) {
        // Use forking for true parallelism
        $pids = [];

        for ($w = 0; $w < $numWorkers; $w++) {
            $pid = pcntl_fork();

            if ($pid == -1) {
                die("Could not fork");
            } elseif ($pid == 0) {
                // Child process
                $client = new Client('127.0.0.1', $port);
                $client->auth('_system', 'admin', $password);

                for ($i = 0; $i < $insertsPerWorker; $i++) {
                    $client->insert($db, $col, [
                        'worker' => $w,
                        'id' => $i,
                        'data' => 'parallel benchmark data'
                    ]);
                }
                exit(0);
            } else {
                $pids[] = $pid;
            }
        }

        // Wait for all children
        foreach ($pids as $pid) {
            pcntl_waitpid($pid, $status);
        }
    } else {
        // Fallback: sequential execution (no true parallelism)
        $client = new Client('127.0.0.1', $port);
        $client->auth('_system', 'admin', $password);

        for ($i = 0; $i < $totalInserts; $i++) {
            $client->insert($db, $col, [
                'id' => $i,
                'data' => 'parallel benchmark data'
            ]);
        }
    }

    $endTime = microtime(true);
    $duration = $endTime - $startTime;
    $opsPerSec = $totalInserts / $duration;

    echo "PHP_PARALLEL_BENCH_RESULT:" . round($opsPerSec, 2) . "\n";
}

run_parallel_benchmark();
