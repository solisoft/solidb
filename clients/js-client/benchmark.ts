import { Client } from './src/Client';

async function runBenchmark() {
    const client = new Client('127.0.0.1', 9999);
    await client.connect();
    await client.auth('_system', 'admin', 'admin');

    const db = 'bench_db';
    const col = 'js_bench';

    try {
        await client.query('_system', `CREATE DATABASE ${db}`);
    } catch (e) { }
    try {
        await client.query(db, `CREATE COLLECTION ${col}`);
    } catch (e) { }

    const iterations = 1000;

    const startTime = Date.now();
    for (let i = 0; i < iterations; i++) {
        await client.insert(db, col, { id: i, data: 'benchmark data content' });
    }
    const endTime = Date.now();

    const duration = (endTime - startTime) / 1000;
    const opsPerSec = iterations / duration;

    console.log(`JS_BENCH_RESULT:${opsPerSec.toFixed(2)}`);
    client.close();
}

runBenchmark();
