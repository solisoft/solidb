import { Client } from './src/Client';

async function runParallelBenchmark() {
    // @ts-ignore - Bun globals
    const env = typeof Bun !== 'undefined' ? Bun.env : process.env;
    const port = parseInt(env.SOLIDB_PORT || '9998');
    const password = env.SOLIDB_PASSWORD || 'password';

    const numWorkers = 16;
    const totalInserts = 10000;
    const insertsPerWorker = totalInserts / numWorkers;

    const db = 'bench_db';
    const col = 'js_parallel_bench';

    // Setup: create database and collection
    const setupClient = new Client('127.0.0.1', port);
    await setupClient.connect();
    await setupClient.auth('_system', 'admin', password);
    try { await setupClient.createDatabase(db); } catch (e) { }
    try { await setupClient.createCollection(db, col); } catch (e) { }
    setupClient.close();

    const startTime = Date.now();

    // Create worker promises
    const workers = [];
    for (let w = 0; w < numWorkers; w++) {
        workers.push((async (workerId: number) => {
            const client = new Client('127.0.0.1', port);
            await client.connect();
            await client.auth('_system', 'admin', password);

            for (let i = 0; i < insertsPerWorker; i++) {
                await client.insert(db, col, {
                    worker: workerId,
                    id: i,
                    data: 'parallel benchmark data'
                });
            }

            client.close();
        })(w));
    }

    // Wait for all workers
    await Promise.all(workers);

    const endTime = Date.now();
    const duration = (endTime - startTime) / 1000;
    const opsPerSec = totalInserts / duration;

    console.log(`JS_PARALLEL_BENCH_RESULT:${opsPerSec.toFixed(2)}`);
}

runParallelBenchmark();
