<?php

// Transactions over the driver, against a running server (card #793):
//
//   SOLIDB_PORT=6745 php tests/transaction_test.php
//
// The client used to send a transaction's writes as plain commands, applied at
// once and kept after a rollback.

require_once __DIR__ . '/../vendor/autoload.php';

use SoliDB\Client;
use SoliDB\Exception\DriverException;

$port = getenv('SOLIDB_PORT') ? (int) getenv('SOLIDB_PORT') : 6745;
$user = getenv('SOLIDB_USER') ?: 'admin';
$password = getenv('SOLIDB_PASSWORD') ?: 'admin';
$db = 'php_tx_test_db';

function connected(int $port, string $user, string $password): Client
{
    $c = new Client('127.0.0.1', $port);
    $c->connect();
    $c->auth('_system', $user, $password);
    return $c;
}

function check(bool $condition, string $what): void
{
    if (!$condition) {
        fwrite(STDERR, "FAILED: $what\n");
        exit(1);
    }
}

function n(Client $c, string $db, string $key)
{
    try {
        $doc = $c->get($db, 'docs', $key);
        return $doc['n'] ?? null;
    } catch (DriverException $e) {
        return null;
    }
}

function refused(callable $f, string $type, string $needle = ''): bool
{
    try {
        $f();
    } catch (DriverException $e) {
        return $e->getErrorType() === $type && ($needle === '' || strpos($e->getMessage(), $needle) !== false);
    }
    return false;
}

$c = connected($port, $user, $password);
$observer = connected($port, $user, $password);
try {
    $c->deleteDatabase($db);
} catch (DriverException $e) {
}
$c->createDatabase($db);
$c->createCollection($db, 'docs');
$c->insert($db, 'docs', ['n' => 1], 'base');
$c->insert($db, 'docs', ['n' => 1], 'gone');

// A rollback leaves nothing, and nothing shows before it.
$tx = $c->beginTransaction($db);
check(is_string($tx) && $tx !== '', 'beginTransaction returns the id');
check($c->inTransaction(), 'inTransaction after begin');
$c->insert($db, 'docs', ['n' => 7], 'new');
$c->update($db, 'docs', 'base', ['n' => 2]);
$c->delete($db, 'docs', 'gone');
check(n($observer, $db, 'new') === null, 'staged insert invisible before the end');
check(n($observer, $db, 'base') === 1, 'staged update invisible before the end');
$c->rollbackTransaction($tx);
check(!$c->inTransaction(), 'no transaction after the rollback');
check(n($observer, $db, 'new') === null, 'insert rolled back');
check(n($observer, $db, 'base') === 1, 'update rolled back');
check(n($observer, $db, 'gone') === 1, 'delete rolled back');
echo "rollback OK\n";

// A commit applies every write at once.
$tx = $c->beginTransaction($db);
$c->insert($db, 'docs', ['n' => 3], 'a');
$c->update($db, 'docs', 'base', ['n' => 2]);
$c->delete($db, 'docs', 'gone');
check($c->query($db, "FOR d IN docs FILTER d._key == 'base' RETURN d.n") === [1], 'read-only query runs on committed data');
check(n($observer, $db, 'a') === null, 'staged insert invisible before the commit');
$c->commitTransaction($tx);
check(n($observer, $db, 'a') === 3, 'insert committed');
check(n($observer, $db, 'base') === 2, 'update committed');
check(n($observer, $db, 'gone') === null, 'delete committed');
echo "commit OK\n";

// A query that writes is staged (the rollback discards it); a second begin
// is refused.
$tx = $c->beginTransaction($db);
$c->query($db, "INSERT {_key: 'q', n: 1} INTO docs");
check(n($observer, $db, 'q') === null, 'a staged query is invisible before the end');
check(refused(function () use ($c, $db) {
    $c->beginTransaction($db);
}, 'transaction_error'), 'a second begin is refused');
$c->rollbackTransaction($tx);
check(n($observer, $db, 'q') === null, 'the rollback discarded the staged query');
check(refused(function () use ($c, $tx) {
    $c->commitTransaction($tx);
}, 'transaction_error'), 'commit after rollback is refused');
echo "refusals OK\n";

$c->deleteDatabase($db);
$c->close();
$observer->close();
echo "ALL TRANSACTION TESTS PASSED\n";
