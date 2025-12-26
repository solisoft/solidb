import { Client } from './src/Client';

async function runBenchmark() {
    // @ts-ignore - Bun globals
    const env = typeof Bun !== 'undefined' ? Bun.env : process.env;
    const port = parseInt(env.SOLIDB_PORT || '9998');
    const password = env.SOLIDB_PASSWORD || 'password';

    const client = new Client('127.0.0.1', port);
    await client.connect();
    await client.auth('_system', 'admin', password);

    const db = 'bench_db';
    const col = 'js_bench';

    try {
        await client.createDatabase(db);
    } catch (e) { }
    try {
        await client.createCollection(db, col);
    } catch (e) { }

    const iterations = 1000;

    // INSERT BENCHMARK
    const startTime = Date.now();
    for (let i = 0; i < iterations; i++) {
        const key = `bench_${i}`;
        await client.insert(db, col, { id: i, data: 'benchmark data content' }, key);
    }
    const insertEndTime = Date.now();
    const insertDuration = (insertEndTime - startTime) / 1000;
    const insertOpsPerSec = iterations / insertDuration;
    console.log(`JS_BENCH_RESULT:${insertOpsPerSec.toFixed(2)}`);

    // READ BENCHMARK
    const readStartTime = Date.now();
    for (let i = 0; i < iterations; i++) {
        const key = `bench_${i}`;
        await client.get(db, col, key);
    }
    const readEndTime = Date.now();
    const readDuration = (readEndTime - readStartTime) / 1000;
    const readOpsPerSec = iterations / readDuration;
    console.log(`JS_READ_BENCH_RESULT:${readOpsPerSec.toFixed(2)}`);

    client.close();
}

runBenchmark();
