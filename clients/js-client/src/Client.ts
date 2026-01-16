import * as net from 'net';
import { encode, decode } from '@msgpack/msgpack';
import { ConnectionError, ServerError, ProtocolError } from './errors';

const MAGIC_HEADER = Buffer.from('solidb-drv-v1\0');
// 16MB limit
const MAX_MESSAGE_SIZE = 16 * 1024 * 1024;

// Import sub-clients
import { ScriptsClient } from './sub-clients/ScriptsClient';
import { JobsClient } from './sub-clients/JobsClient';
import { CronClient } from './sub-clients/CronClient';
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
    private socket: net.Socket | null = null;
    private connected: boolean = false;
    private buffer: Buffer = Buffer.alloc(0);
    private nextMessageLength: number | null = null;
    private _database: string = '';

    // Request queue to handle responses in order
    private requestQueue: Array<{
        resolve: (val: any) => void;
        reject: (err: any) => void;
    }> = [];

    // Sub-clients
    public readonly scripts: ScriptsClient;
    public readonly jobs: JobsClient;
    public readonly cron: CronClient;
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
        private port: number = 6745
    ) {
        // Initialize sub-clients
        this.scripts = new ScriptsClient(this);
        this.jobs = new JobsClient(this);
        this.cron = new CronClient(this);
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

    public async connect(): Promise<void> {
        if (this.connected) return;

        return new Promise((resolve, reject) => {
            this.socket = new net.Socket();

            this.socket.on('connect', () => {
                this.socket?.write(MAGIC_HEADER);
                this.connected = true;
                resolve();
            });

            this.socket.on('data', (data) => this.handleData(data));

            this.socket.on('error', (err) => {
                this.connected = false;
                if (this.requestQueue.length === 0) {
                    reject(new ConnectionError(err.message));
                } else {
                    this.failAll(new ConnectionError(err.message));
                }
            });

            this.socket.on('close', () => {
                this.connected = false;
                this.failAll(new ConnectionError("Connection closed"));
            });

            this.socket.connect(this.port, this.host);
        });
    }

    public close(): void {
        if (this.socket) {
            this.socket.destroy();
            this.socket = null;
        }
        this.connected = false;
    }

    private handleData(chunk: Buffer) {
        this.buffer = Buffer.concat([this.buffer, chunk]);

        while (true) {
            if (this.nextMessageLength === null) {
                if (this.buffer.length >= 4) {
                    this.nextMessageLength = this.buffer.readUInt32BE(0);
                    this.buffer = this.buffer.subarray(4);

                    if (this.nextMessageLength > MAX_MESSAGE_SIZE) {
                        const err = new ProtocolError(`Message too large: ${this.nextMessageLength}`);
                        this.close();
                        this.failAll(err);
                        return;
                    }
                } else {
                    break;
                }
            }

            if (this.nextMessageLength !== null) {
                if (this.buffer.length >= this.nextMessageLength) {
                    const payload = this.buffer.subarray(0, this.nextMessageLength);
                    this.buffer = this.buffer.subarray(this.nextMessageLength);
                    this.nextMessageLength = null;

                    this.processMessage(payload);
                } else {
                    break;
                }
            }
        }
    }

    private processMessage(payload: Buffer) {
        const req = this.requestQueue.shift();
        if (!req) return;

        try {
            const response = decode(payload) as any;

            // Handle Tuple [status, body]
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
            // Handle Map
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

    private failAll(err: Error) {
        while (this.requestQueue.length > 0) {
            const req = this.requestQueue.shift();
            req?.reject(err);
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

    // --- Internal Command Method (exposed for sub-clients) ---

    public async sendCommand(cmd: string, args: Record<string, any> = {}): Promise<any> {
        if (!this.connected) await this.connect();

        return new Promise((resolve, reject) => {
            const command = { cmd, ...args };
            try {
                const payload = encode(command);
                const header = Buffer.alloc(4);
                header.writeUInt32BE(payload.length, 0);

                this.requestQueue.push({ resolve, reject });
                this.socket?.write(Buffer.concat([header, Buffer.from(payload)]));
            } catch (e: any) {
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
