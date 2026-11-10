import * as net from 'net';
import { encode, decode } from '@msgpack/msgpack';
import { ConnectionError, ServerError, ProtocolError, TransactionError } from './errors';

const MAGIC_HEADER = Buffer.from('solidb-drv-v1\0');
const MAX_MESSAGE_SIZE = 16 * 1024 * 1024;
const DEFAULT_POOL_SIZE = 4;
const SOCKET_BUFFER_SIZE = 1024 * 1024;

// Inside a transaction the server stages these when they arrive wrapped in a
// `transaction_command` on the connection that began it (a query reads
// committed data; the writes of one that writes are staged too, on servers
// since 2.9.2; 2.9.1 and earlier refuse it).
const STAGED_IN_TRANSACTION = new Set(['insert', 'update', 'delete', 'query']);
// These cannot be staged: refused rather than applied outside the transaction.
const REFUSED_IN_TRANSACTION = new Set(['bulk_insert', 'batch', 'begin_transaction']);

/** The server's error as text: a string, or `{Kind: "message"}`. */
function errorMessage(body: any): string {
    if (typeof body === 'string') return body;
    if (body && typeof body === 'object') {
        const vals = Object.values(body);
        if (vals.length > 0) return String(vals[0]);
        return JSON.stringify(body);
    }
    return 'Unknown error';
}

interface PooledConnection {
    socket: net.Socket;
    buffer: Buffer;
    nextMessageLength: number | null;
    requestQueue: Array<{
        resolve: (val: any) => void;
        reject: (err: any) => void;
    }>;
    inUse: boolean;
}

import { ScriptsClient } from './sub-clients/ScriptsClient';
import { TriggersClient } from './sub-clients/TriggersClient';
import { EnvClient } from './sub-clients/EnvClient';
import { RolesClient } from './sub-clients/RolesClient';
import { UsersClient } from './sub-clients/UsersClient';
import { ApiKeysClient } from './sub-clients/ApiKeysClient';
import { ClusterClient } from './sub-clients/ClusterClient';
import { CollectionsClient } from './sub-clients/CollectionsClient';
import { IndexesClient } from './sub-clients/IndexesClient';
import { GeoClient } from './sub-clients/GeoClient';
import { VectorClient } from './sub-clients/VectorClient';
import { TTLClient } from './sub-clients/TTLClient';
import { ColumnarClient } from './sub-clients/ColumnarClient';

export class Client {
    private pool: PooledConnection[] = [];
    private poolSize: number = DEFAULT_POOL_SIZE;
    private nextConnIndex: number = 0;
    private connected: boolean = false;
    private _database: string = '';
    // The open transaction and the pooled connection it lives on: the server
    // keeps a driver transaction per connection.
    private tx: { id: string; conn: PooledConnection } | null = null;

    // Sub-clients
    public readonly scripts: ScriptsClient;
    public readonly triggers: TriggersClient;
    public readonly env: EnvClient;
    public readonly roles: RolesClient;
    public readonly users: UsersClient;
    public readonly apiKeys: ApiKeysClient;
    public readonly cluster: ClusterClient;
    public readonly collections: CollectionsClient;
    public readonly indexes: IndexesClient;
    public readonly geo: GeoClient;
    public readonly vector: VectorClient;
    public readonly ttl: TTLClient;
    public readonly columnar: ColumnarClient;

    constructor(
        private host: string = '127.0.0.1',
        private port: number = 6745,
        poolSize: number = DEFAULT_POOL_SIZE
    ) {
        this.poolSize = poolSize;
        this.scripts = new ScriptsClient(this);
        this.triggers = new TriggersClient(this);
        this.env = new EnvClient(this);
        this.roles = new RolesClient(this);
        this.users = new UsersClient(this);
        this.apiKeys = new ApiKeysClient(this);
        this.cluster = new ClusterClient(this);
        this.collections = new CollectionsClient(this);
        this.indexes = new IndexesClient(this);
        this.geo = new GeoClient(this);
        this.vector = new VectorClient(this);
        this.ttl = new TTLClient(this);
        this.columnar = new ColumnarClient(this);
    }

    private createConnection(): Promise<PooledConnection> {
        return new Promise((resolve, reject) => {
            const socket = new net.Socket() as net.Socket;
            (socket as any).setNoDelay(true);
            (socket as any).setKeepAlive(true, 30000);

            const conn: PooledConnection = {
                socket,
                buffer: Buffer.alloc(0),
                nextMessageLength: null,
                requestQueue: [],
                inUse: false
            };

            socket.on('connect', () => {
                socket.write(MAGIC_HEADER);
                resolve(conn);
            });

            socket.on('data', (data) => this.handleData(data, conn));

            socket.on('error', (err) => {
                conn.inUse = false;
                this.connected = false;
                while (conn.requestQueue.length > 0) {
                    const req = conn.requestQueue.shift();
                    req?.reject(new ConnectionError(err.message));
                }
            });

            socket.on('close', () => {
                conn.inUse = false;
                this.connected = false;
                while (conn.requestQueue.length > 0) {
                    const req = conn.requestQueue.shift();
                    req?.reject(new ConnectionError("Connection closed"));
                }
            });

            socket.connect(this.port, this.host);
        });
    }

    public async connect(): Promise<void> {
        if (this.connected && this.pool.length > 0) return;

        this.pool = [];
        const connections = await Promise.all(
            Array.from({ length: this.poolSize }, () => this.createConnection())
        );
        this.pool = connections;
        this.connected = true;
    }

    public close(): void {
        for (const conn of this.pool) {
            conn.socket.destroy();
        }
        this.pool = [];
        this.connected = false;
    }

    private handleData(chunk: Buffer, conn: PooledConnection) {
        const newLength = conn.buffer.length + chunk.length;
        // Append the chunk. The previous version copied it past the end of
        // the existing buffer whenever the chunk was the smaller of the two,
        // which writes nothing: any response spanning three or more socket
        // reads (from ~64 KB) lost data, failed to decode, and left the
        // connection waiting forever.
        conn.buffer =
            conn.buffer.length === 0 ? chunk : Buffer.concat([conn.buffer, chunk], newLength);

        let offset = 0;
        while (true) {
            if (conn.nextMessageLength === null) {
                if (newLength - offset >= 4) {
                    conn.nextMessageLength = conn.buffer.readUInt32BE(offset);
                    offset += 4;

                    if (conn.nextMessageLength > MAX_MESSAGE_SIZE) {
                        const err = new ProtocolError(`Message too large: ${conn.nextMessageLength}`);
                        conn.socket.destroy();
                        while (conn.requestQueue.length > 0) {
                            const req = conn.requestQueue.shift();
                            req?.reject(err);
                        }
                        return;
                    }
                } else {
                    break;
                }
            }

            if (conn.nextMessageLength !== null) {
                if (newLength - offset >= conn.nextMessageLength) {
                    const payload = conn.buffer.subarray(offset, offset + conn.nextMessageLength);
                    offset += conn.nextMessageLength;
                    conn.nextMessageLength = null;

                    this.processMessage(payload, conn);
                } else {
                    break;
                }
            }
        }

        if (offset > 0 && offset < newLength) {
            conn.buffer = conn.buffer.subarray(offset);
        } else if (offset === newLength) {
            conn.buffer = Buffer.alloc(0);
        }
    }

    private processMessage(payload: Buffer, conn: PooledConnection) {
        const req = conn.requestQueue.shift();
        if (!req) return;
        conn.inUse = false;

        try {
            const response = decode(payload) as any;

            if (Array.isArray(response) && response.length >= 1 && typeof response[0] === 'string') {
                const status = response[0];
                const body = response[1];

                if (status === 'ok' || status === 'pong') {
                    req.resolve(body);
                } else if (status === 'error') {
                    req.reject(new ServerError(errorMessage(body)));
                } else {
                    req.resolve(body);
                }
                return;
            }
            if (response && typeof response === 'object' && !Array.isArray(response)) {
                if (response.status === 'error') {
                    // `{Kind: "message"}`: was passed to Error as an object,
                    // and read "[object Object]".
                    req.reject(new ServerError(errorMessage(response.error)));
                } else {
                    // A begin answers with `tx_id` and no data.
                    req.resolve(response.data !== undefined ? response.data : response.tx_id);
                }
                return;
            }

            req.resolve(response);

        } catch (e: any) {
            req.reject(new ProtocolError("Failed to deserialize: " + e.message));
        }
    }

    // --- Database Context ---

    public useDatabase(name: string): this {
        this._database = name;
        return this;
    }

    public get database(): string {
        return this._database;
    }

    private getNextConnection(): PooledConnection {
        const start = this.nextConnIndex;
        while (this.pool[this.nextConnIndex].inUse) {
            this.nextConnIndex = (this.nextConnIndex + 1) % this.pool.length;
            if (this.nextConnIndex === start) {
                break;
            }
        }
        const conn = this.pool[this.nextConnIndex];
        conn.inUse = true;
        this.nextConnIndex = (this.nextConnIndex + 1) % this.pool.length;
        return conn;
    }

    public async sendCommand(cmd: string, args: Record<string, any> = {}): Promise<any> {
        if (!this.connected || this.pool.length === 0) {
            await this.connect();
        }

        if (this.tx) {
            if (STAGED_IN_TRANSACTION.has(cmd)) {
                return this.sendOn(this.tx.conn, {
                    cmd: 'transaction_command',
                    tx_id: this.tx.id,
                    command: { cmd, ...args },
                });
            }
            if (REFUSED_IN_TRANSACTION.has(cmd)) {
                throw new TransactionError(
                    `${cmd} cannot run inside a transaction: commit or roll back first`
                );
            }
        }
        return this.sendOn(this.getNextConnection(), { cmd, ...args });
    }

    private sendOn(conn: PooledConnection, command: Record<string, any>): Promise<any> {
        conn.inUse = true;
        return new Promise((resolve, reject) => {
            try {
                const payload = encode(command);
                const header = Buffer.alloc(4);
                header.writeUInt32BE(payload.length, 0);

                conn.requestQueue.push({ resolve, reject });
                conn.socket.write(header);
                conn.socket.write(Buffer.from(payload));
            } catch (e: any) {
                conn.inUse = false;
                reject(e);
            }
        });
    }

    // --- Public API ---

    public async ping(): Promise<void> {
        await this.sendCommand('ping');
    }

    // Authentication is per connection: every pooled one must log in, or the
    // commands that land on the others are refused (3 in 4 with the default
    // pool, when only the first one was).
    public async auth(database: string, username: string, password: string): Promise<void> {
        await this.authAll({ cmd: 'auth', database, username, password });
    }

    public async authWithApiKey(database: string, apiKey: string): Promise<void> {
        await this.authAll({ cmd: 'auth', database, username: '', password: '', api_key: apiKey });
    }

    private async authAll(command: Record<string, any>): Promise<void> {
        if (!this.connected || this.pool.length === 0) {
            await this.connect();
        }
        await Promise.all(this.pool.map((conn) => this.sendOn(conn, command)));
    }

    // Database
    public async listDatabases(): Promise<string[]> {
        return (await this.sendCommand('list_databases')) || [];
    }

    public async createDatabase(name: string): Promise<void> {
        await this.sendCommand('create_database', { name });
    }

    public async deleteDatabase(name: string): Promise<void> {
        await this.sendCommand('delete_database', { name });
    }

    // Collection
    public async listCollections(database: string): Promise<string[]> {
        return (await this.sendCommand('list_collections', { database })) || [];
    }

    public async createCollection(database: string, name: string, type?: string): Promise<void> {
        await this.sendCommand('create_collection', { database, name, type });
    }

    public async deleteCollection(database: string, name: string): Promise<void> {
        await this.sendCommand('delete_collection', { database, name });
    }

    // Document
    public async insert(database: string, collection: string, document: any, key?: string): Promise<any> {
        return await this.sendCommand('insert', { database, collection, document, key });
    }

    public async get(database: string, collection: string, key: string): Promise<any> {
        return await this.sendCommand('get', { database, collection, key });
    }

    public async update(database: string, collection: string, key: string, document: any, merge: boolean = true): Promise<void> {
        await this.sendCommand('update', { database, collection, key, document, merge });
    }

    public async delete(database: string, collection: string, key: string): Promise<void> {
        await this.sendCommand('delete', { database, collection, key });
    }

    public async list(database: string, collection: string, limit: number = 50, offset: number = 0): Promise<any[]> {
        return (await this.sendCommand('list', { database, collection, limit, offset })) || [];
    }

    // Query
    public async query(database: string, sdbql: string, bindVars: Record<string, any> = {}): Promise<any[]> {
        return (await this.sendCommand('query', { database, sdbql, bind_vars: bindVars })) || [];
    }

    public async explain(database: string, sdbql: string, bindVars: Record<string, any> = {}): Promise<any> {
        return (await this.sendCommand('explain', { database, sdbql, bind_vars: bindVars })) || {};
    }

    // Transactions
    /**
     * Open a transaction on one of the pool's connections and return its id.
     *
     * Until it is committed or rolled back, `insert`, `update` and `delete`
     * are staged in it and applied at commit, and a read-only `query` runs on
     * committed data (it does not see the transaction's own writes); a query
     * that writes, `batch` and a second `beginTransaction` are refused. `get`
     * is not part of the transaction: it reads committed data. One transaction
     * at a time per client.
     */
    public async beginTransaction(database: string, isolationLevel: string = 'read_committed'): Promise<string> {
        if (this.tx) {
            throw new TransactionError('a transaction is already open on this client: commit or roll it back first');
        }
        if (!this.connected || this.pool.length === 0) {
            await this.connect();
        }
        const conn = this.getNextConnection();
        const id = await this.sendOn(conn, { cmd: 'begin_transaction', database, isolation_level: isolationLevel });
        if (typeof id !== 'string') {
            throw new ProtocolError('begin_transaction: no transaction id in the response');
        }
        this.tx = { id, conn };
        return id;
    }

    /** Commit the open transaction, on the connection that began it. */
    public async commitTransaction(txId: string): Promise<void> {
        await this.endTransaction('commit_transaction', txId);
    }

    /** Roll back the open transaction: nothing it staged is applied. */
    public async rollbackTransaction(txId: string): Promise<void> {
        await this.endTransaction('rollback_transaction', txId);
    }

    private async endTransaction(cmd: string, txId: string): Promise<void> {
        const tx = this.tx;
        if (!tx || tx.id !== txId) {
            throw new TransactionError(`transaction ${txId} is not open on this client`);
        }
        // Whatever the outcome, the client has no open transaction afterwards.
        this.tx = null;
        await this.sendOn(tx.conn, { cmd, tx_id: txId });
    }

    public get inTransaction(): boolean {
        return this.tx !== null;
    }
}
