// Transactions over the driver, against a running server (card #793):
//
//   SOLIDB_PORT=6745 npx ts-node tests/transaction.test.ts
//
// The client used to send a transaction's writes as plain commands — applied
// at once and kept after a rollback — and its default pool of four
// connections lost the transaction (and the login) on three of them.
import * as assert from 'assert';
import { Client, TransactionError } from '../src';

const PORT = parseInt(process.env.SOLIDB_PORT || '6745');
const USER = process.env.SOLIDB_USER || 'admin';
const PASSWORD = process.env.SOLIDB_PASSWORD || 'admin';
const DB = 'js_tx_test_db';

async function client(): Promise<Client> {
    const c = new Client('127.0.0.1', PORT); // the default pool of four
    await c.connect();
    await c.auth('_system', USER, PASSWORD);
    return c;
}

async function n(c: Client, key: string): Promise<number | null> {
    try {
        return (await c.get(DB, 'docs', key)).n;
    } catch {
        return null;
    }
}

async function run() {
    const c = await client();
    const observer = await client();
    try { await c.deleteDatabase(DB); } catch { }
    await c.createDatabase(DB);
    await c.createCollection(DB, 'docs');
    await c.insert(DB, 'docs', { n: 1 }, 'base');
    await c.insert(DB, 'docs', { n: 1 }, 'gone');

    // A rollback leaves nothing, and nothing shows before it.
    let tx = await c.beginTransaction(DB);
    assert.ok(typeof tx === 'string' && tx.length > 0, 'beginTransaction returns the id');
    assert.ok(c.inTransaction);
    await c.insert(DB, 'docs', { n: 7 }, 'new');
    await c.update(DB, 'docs', 'base', { n: 2 });
    await c.delete(DB, 'docs', 'gone');
    assert.strictEqual(await n(observer, 'new'), null);
    assert.strictEqual(await n(observer, 'base'), 1);
    await c.rollbackTransaction(tx);
    assert.ok(!c.inTransaction);
    assert.strictEqual(await n(observer, 'new'), null);
    assert.strictEqual(await n(observer, 'base'), 1);
    assert.strictEqual(await n(observer, 'gone'), 1);
    console.log('rollback OK');

    // A commit applies every write at once, whatever else goes out in between
    // on the other pooled connections.
    tx = await c.beginTransaction(DB);
    await c.insert(DB, 'docs', { n: 3 }, 'a');
    assert.strictEqual((await c.get(DB, 'docs', 'base')).n, 1);
    await c.update(DB, 'docs', 'base', { n: 2 });
    assert.strictEqual((await c.get(DB, 'docs', 'base')).n, 1);
    await c.delete(DB, 'docs', 'gone');
    assert.deepStrictEqual(await c.query(DB, "FOR d IN docs FILTER d._key == 'base' RETURN d.n"), [1]);
    assert.strictEqual(await n(observer, 'a'), null);
    await c.commitTransaction(tx);
    assert.strictEqual(await n(observer, 'a'), 3);
    assert.strictEqual(await n(observer, 'base'), 2);
    assert.strictEqual(await n(observer, 'gone'), null);
    console.log('commit OK');

    // A query that writes is staged (the rollback discards it); a second
    // begin is refused.
    tx = await c.beginTransaction(DB);
    await c.query(DB, "INSERT {_key: 'q', n: 1} INTO docs");
    assert.strictEqual(await n(observer, 'q'), null);
    await assert.rejects(c.beginTransaction(DB), TransactionError);
    await c.rollbackTransaction(tx);
    assert.strictEqual(await n(observer, 'q'), null);
    await assert.rejects(c.commitTransaction(tx), TransactionError);
    console.log('refusals OK');

    // Every pooled connection is logged in: four requests in a row succeed.
    for (let i = 0; i < 8; i++) {
        assert.ok((await c.listDatabases()).includes(DB));
    }
    console.log('pooled auth OK');

    await c.deleteDatabase(DB);
    c.close();
    observer.close();
    console.log('ALL TRANSACTION TESTS PASSED');
}

run().catch((e) => {
    console.error('TEST FAILED:', e);
    process.exit(1);
});
