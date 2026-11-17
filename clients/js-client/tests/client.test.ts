import * as assert from 'assert';
import { Client, ServerError } from '../src';
// Using generic test function wrapper as 'node:test' might require specific runner call.
// But we can use simple async function execution.

const PORT = parseInt(process.env.SOLIDB_PORT || '6745');
const DB = 'js_test_db';

async function runTests() {
    console.log('Running JS Client Tests...');
    const client = new Client('127.0.0.1', PORT);

    try {
        console.log('Connecting...');
        await client.connect();
        await client.ping();
        console.log('Connected + Ping OK');

        try { await client.auth('_system', 'admin', 'admin'); } catch (e) { }

        // Disclaimer: Cleanup might fail if db doesn't exist
        try { await client.deleteDatabase(DB); } catch (e) { }

        await client.createDatabase(DB);
        await client.createCollection(DB, 'users');
        console.log('Database/Collection Created OK');

        // CRUD
        const doc = await client.insert(DB, 'users', { name: 'NodeJS', ver: 20 });
        assert.ok(doc._key, 'Insert should return _key');
        console.log('Insert OK', doc._key);

        const fetched = await client.get(DB, 'users', doc._key);
        assert.strictEqual(fetched.name, 'NodeJS');
        console.log('Get OK');

        await client.update(DB, 'users', doc._key, { ver: 22 });
        const updated = await client.get(DB, 'users', doc._key);
        assert.strictEqual(updated.ver, 22);
        console.log('Update OK');

        const list = await client.list(DB, 'users');
        assert.ok(list.length >= 1);
        console.log('List OK');

        // Text SDBQL
        const queryRes = await client.query(DB, 'FOR u IN users RETURN u');
        assert.ok(queryRes.length >= 1);
        console.log('Query OK');

        // Transaction
        const txId = await client.beginTransaction(DB);
        assert.ok(typeof txId === 'string' && txId.length > 0);
        await client.commitTransaction(txId);
        console.log('Transaction OK');

        await client.delete(DB, 'users', doc._key);
        try {
            await client.get(DB, 'users', doc._key);
            assert.fail('Should throw error after delete');
        } catch (e) {
            assert.ok(e instanceof ServerError);
        }
        console.log('Delete OK');

        // Foreign keys
        await client.createCollection(DB, 'customers');
        await client.createCollection(DB, 'orders');
        await client.insert(DB, 'customers', { _key: 'c1', name: 'Ada' });
        await client.insert(DB, 'orders', { _key: 'o1', customer: 'c1' });
        await client.insert(DB, 'orders', { _key: 'o2', customer: 'ghost' });
        client.useDatabase(DB);
        const orphans = await client.indexes.checkForeignKey('orders', 'customer', 'customers');
        assert.strictEqual(orphans.count, 1);
        assert.strictEqual(orphans.sample[0]._key, 'o2');
        await assert.rejects(client.indexes.createForeignKey('orders', 'customer', 'customers'));
        await client.delete(DB, 'orders', 'o2');
        const fk = await client.indexes.createForeignKey('orders', 'customer', 'customers', {
            onDelete: 'cascade',
            name: 'fk_customer'
        });
        assert.strictEqual(fk.name, 'fk_customer');
        assert.strictEqual(fk.on_delete, 'cascade');
        assert.strictEqual((await client.indexes.listForeignKeys('orders')).foreign_keys[0].field, 'customer');
        assert.strictEqual(
            (await client.indexes.listForeignKeys('customers')).referenced_by[0].collection,
            'orders'
        );
        await assert.rejects(client.insert(DB, 'orders', { _key: 'o3', customer: 'nobody' }));
        await client.delete(DB, 'customers', 'c1');
        assert.deepStrictEqual(await client.query(DB, 'FOR o IN orders RETURN o._key'), []);
        await client.indexes.deleteForeignKey('orders', 'fk_customer');
        assert.deepStrictEqual((await client.indexes.listForeignKeys('orders')).foreign_keys, []);
        console.log('Foreign keys OK');

    } catch (e: any) {
        console.error('TEST FAILED:', e);
        process.exit(1);
    } finally {
        try { await client.deleteDatabase(DB); } catch (e) { }
        client.close();
    }
}

runTests();
