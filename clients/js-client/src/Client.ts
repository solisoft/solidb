import * as net from 'net';
import { encode, decode } from '@msgpack/msgpack';
import { ConnectionError, ServerError, ProtocolError } from './errors';

const MAGIC_HEADER = Buffer.from('solidb-drv-v1\0');
const MAX_MESSAGE_SIZE = 16 * 1024 * 1024;
const DEFAULT_POOL_SIZE = 4;
const SOCKET_BUFFER_SIZE = 1024 * 1024;

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
        if (conn.buffer.length === 0) {
            conn.buffer = Buffer.allocUnsafe(newLength);
            chunk.copy(conn.buffer);
        } else if (conn.buffer.length >= chunk.length) {
            chunk.copy(conn.buffer, conn.buffer.length);
        } else {
            const newBuffer = Buffer.allocUnsafe(newLength);
            conn.buffer.copy(newBuffer);
            chunk.copy(newBuffer, conn.buffer.length);
            conn.buffer = newBuffer;
        }

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
                    let msg = "Unknown error";
                    if (typeof body === 'string') msg = body;
                    else if (typeof body === 'object' && body) {
                        const vals = Object.values(body);
                        if (vals.length > 0) msg = String(vals[0]);
                        else msg = JSON.stringify(body);
                    }
                    req.reject(new ServerError(msg));
                } else {
                    req.resolve(body);
                }
                return;
            }
            if (response && typeof response === 'object' && !Array.isArray(response)) {
                if (response.status === 'error') {
                    req.reject(new ServerError(response.error || "Unknown error"));
                } else {
                    req.resolve(response.data);
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

        const conn = this.getNextConnection();

        return new Promise((resolve, reject) => {
            const command = { cmd, ...args };
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

    public async auth(database: string, username: string, password: string): Promise<void> {
        await this.sendCommand('auth', { database, username, password });
    }

    public async authWithApiKey(database: string, apiKey: string): Promise<void> {
        await this.sendCommand('auth', { database, username: '', password: '', api_key: apiKey });
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
    public async beginTransaction(database: string, isolationLevel: string = 'read_committed'): Promise<string> {
        return await this.sendCommand('begin_transaction', { database, isolation_level: isolationLevel });
    }

    public async commitTransaction(txId: string): Promise<void> {
        await this.sendCommand('commit_transaction', { tx_id: txId });
    }

    public async rollbackTransaction(txId: string): Promise<void> {
        await this.sendCommand('rollback_transaction', { tx_id: txId });
    }
}
