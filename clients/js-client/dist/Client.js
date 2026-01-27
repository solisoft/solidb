"use strict";
var __createBinding = (this && this.__createBinding) || (Object.create ? (function(o, m, k, k2) {
    if (k2 === undefined) k2 = k;
    var desc = Object.getOwnPropertyDescriptor(m, k);
    if (!desc || ("get" in desc ? !m.__esModule : desc.writable || desc.configurable)) {
      desc = { enumerable: true, get: function() { return m[k]; } };
    }
    Object.defineProperty(o, k2, desc);
}) : (function(o, m, k, k2) {
    if (k2 === undefined) k2 = k;
    o[k2] = m[k];
}));
var __setModuleDefault = (this && this.__setModuleDefault) || (Object.create ? (function(o, v) {
    Object.defineProperty(o, "default", { enumerable: true, value: v });
}) : function(o, v) {
    o["default"] = v;
});
var __importStar = (this && this.__importStar) || (function () {
    var ownKeys = function(o) {
        ownKeys = Object.getOwnPropertyNames || function (o) {
            var ar = [];
            for (var k in o) if (Object.prototype.hasOwnProperty.call(o, k)) ar[ar.length] = k;
            return ar;
        };
        return ownKeys(o);
    };
    return function (mod) {
        if (mod && mod.__esModule) return mod;
        var result = {};
        if (mod != null) for (var k = ownKeys(mod), i = 0; i < k.length; i++) if (k[i] !== "default") __createBinding(result, mod, k[i]);
        __setModuleDefault(result, mod);
        return result;
    };
})();
Object.defineProperty(exports, "__esModule", { value: true });
exports.Client = void 0;
const net = __importStar(require("net"));
const msgpack_1 = require("@msgpack/msgpack");
const errors_1 = require("./errors");
const MAGIC_HEADER = Buffer.from('solidb-drv-v1\0');
const MAX_MESSAGE_SIZE = 16 * 1024 * 1024;
const DEFAULT_POOL_SIZE = 4;
const SOCKET_BUFFER_SIZE = 1024 * 1024;
const ScriptsClient_1 = require("./sub-clients/ScriptsClient");
const JobsClient_1 = require("./sub-clients/JobsClient");
const CronClient_1 = require("./sub-clients/CronClient");
const TriggersClient_1 = require("./sub-clients/TriggersClient");
const EnvClient_1 = require("./sub-clients/EnvClient");
const RolesClient_1 = require("./sub-clients/RolesClient");
const UsersClient_1 = require("./sub-clients/UsersClient");
const ApiKeysClient_1 = require("./sub-clients/ApiKeysClient");
const ClusterClient_1 = require("./sub-clients/ClusterClient");
const CollectionsClient_1 = require("./sub-clients/CollectionsClient");
const IndexesClient_1 = require("./sub-clients/IndexesClient");
const GeoClient_1 = require("./sub-clients/GeoClient");
const VectorClient_1 = require("./sub-clients/VectorClient");
const TTLClient_1 = require("./sub-clients/TTLClient");
const ColumnarClient_1 = require("./sub-clients/ColumnarClient");
class Client {
    constructor(host = '127.0.0.1', port = 6745, poolSize = DEFAULT_POOL_SIZE) {
        this.host = host;
        this.port = port;
        this.pool = [];
        this.poolSize = DEFAULT_POOL_SIZE;
        this.nextConnIndex = 0;
        this.connected = false;
        this._database = '';
        this.poolSize = poolSize;
        this.scripts = new ScriptsClient_1.ScriptsClient(this);
        this.jobs = new JobsClient_1.JobsClient(this);
        this.cron = new CronClient_1.CronClient(this);
        this.triggers = new TriggersClient_1.TriggersClient(this);
        this.env = new EnvClient_1.EnvClient(this);
        this.roles = new RolesClient_1.RolesClient(this);
        this.users = new UsersClient_1.UsersClient(this);
        this.apiKeys = new ApiKeysClient_1.ApiKeysClient(this);
        this.cluster = new ClusterClient_1.ClusterClient(this);
        this.collections = new CollectionsClient_1.CollectionsClient(this);
        this.indexes = new IndexesClient_1.IndexesClient(this);
        this.geo = new GeoClient_1.GeoClient(this);
        this.vector = new VectorClient_1.VectorClient(this);
        this.ttl = new TTLClient_1.TTLClient(this);
        this.columnar = new ColumnarClient_1.ColumnarClient(this);
    }
    createConnection() {
        return new Promise((resolve, reject) => {
            const socket = new net.Socket();
            socket.setNoDelay(true);
            socket.setKeepAlive(true, 30000);
            const conn = {
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
                    req?.reject(new errors_1.ConnectionError(err.message));
                }
            });
            socket.on('close', () => {
                conn.inUse = false;
                this.connected = false;
                while (conn.requestQueue.length > 0) {
                    const req = conn.requestQueue.shift();
                    req?.reject(new errors_1.ConnectionError("Connection closed"));
                }
            });
            socket.connect(this.port, this.host);
        });
    }
    async connect() {
        if (this.connected && this.pool.length > 0)
            return;
        this.pool = [];
        const connections = await Promise.all(Array.from({ length: this.poolSize }, () => this.createConnection()));
        this.pool = connections;
        this.connected = true;
    }
    close() {
        for (const conn of this.pool) {
            conn.socket.destroy();
        }
        this.pool = [];
        this.connected = false;
    }
    handleData(chunk, conn) {
        const newLength = conn.buffer.length + chunk.length;
        if (conn.buffer.length === 0) {
            conn.buffer = Buffer.allocUnsafe(newLength);
            chunk.copy(conn.buffer);
        }
        else if (conn.buffer.length >= chunk.length) {
            chunk.copy(conn.buffer, conn.buffer.length);
        }
        else {
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
                        const err = new errors_1.ProtocolError(`Message too large: ${conn.nextMessageLength}`);
                        conn.socket.destroy();
                        while (conn.requestQueue.length > 0) {
                            const req = conn.requestQueue.shift();
                            req?.reject(err);
                        }
                        return;
                    }
                }
                else {
                    break;
                }
            }
            if (conn.nextMessageLength !== null) {
                if (newLength - offset >= conn.nextMessageLength) {
                    const payload = conn.buffer.subarray(offset, offset + conn.nextMessageLength);
                    offset += conn.nextMessageLength;
                    conn.nextMessageLength = null;
                    this.processMessage(payload, conn);
                }
                else {
                    break;
                }
            }
        }
        if (offset > 0 && offset < newLength) {
            conn.buffer = conn.buffer.subarray(offset);
        }
        else if (offset === newLength) {
            conn.buffer = Buffer.alloc(0);
        }
    }
    processMessage(payload, conn) {
        const req = conn.requestQueue.shift();
        if (!req)
            return;
        conn.inUse = false;
        try {
            const response = (0, msgpack_1.decode)(payload);
            if (Array.isArray(response) && response.length >= 1 && typeof response[0] === 'string') {
                const status = response[0];
                const body = response[1];
                if (status === 'ok' || status === 'pong') {
                    req.resolve(body);
                }
                else if (status === 'error') {
                    let msg = "Unknown error";
                    if (typeof body === 'string')
                        msg = body;
                    else if (typeof body === 'object' && body) {
                        const vals = Object.values(body);
                        if (vals.length > 0)
                            msg = String(vals[0]);
                        else
                            msg = JSON.stringify(body);
                    }
                    req.reject(new errors_1.ServerError(msg));
                }
                else {
                    req.resolve(body);
                }
                return;
            }
            if (response && typeof response === 'object' && !Array.isArray(response)) {
                if (response.status === 'error') {
                    req.reject(new errors_1.ServerError(response.error || "Unknown error"));
                }
                else {
                    req.resolve(response.data);
                }
                return;
            }
            req.resolve(response);
        }
        catch (e) {
            req.reject(new errors_1.ProtocolError("Failed to deserialize: " + e.message));
        }
    }
    // --- Database Context ---
    useDatabase(name) {
        this._database = name;
        return this;
    }
    get database() {
        return this._database;
    }
    getNextConnection() {
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
    async sendCommand(cmd, args = {}) {
        if (!this.connected || this.pool.length === 0) {
            await this.connect();
        }
        const conn = this.getNextConnection();
        return new Promise((resolve, reject) => {
            const command = { cmd, ...args };
            try {
                const payload = (0, msgpack_1.encode)(command);
                const header = Buffer.alloc(4);
                header.writeUInt32BE(payload.length, 0);
                conn.requestQueue.push({ resolve, reject });
                conn.socket.write(header);
                conn.socket.write(Buffer.from(payload));
            }
            catch (e) {
                conn.inUse = false;
                reject(e);
            }
        });
    }
    // --- Public API ---
    async ping() {
        await this.sendCommand('ping');
    }
    async auth(database, username, password) {
        await this.sendCommand('auth', { database, username, password });
    }
    async authWithApiKey(database, apiKey) {
        await this.sendCommand('auth', { database, username: '', password: '', api_key: apiKey });
    }
    // Database
    async listDatabases() {
        return (await this.sendCommand('list_databases')) || [];
    }
    async createDatabase(name) {
        await this.sendCommand('create_database', { name });
    }
    async deleteDatabase(name) {
        await this.sendCommand('delete_database', { name });
    }
    // Collection
    async listCollections(database) {
        return (await this.sendCommand('list_collections', { database })) || [];
    }
    async createCollection(database, name, type) {
        await this.sendCommand('create_collection', { database, name, type });
    }
    async deleteCollection(database, name) {
        await this.sendCommand('delete_collection', { database, name });
    }
    // Document
    async insert(database, collection, document, key) {
        return await this.sendCommand('insert', { database, collection, document, key });
    }
    async get(database, collection, key) {
        return await this.sendCommand('get', { database, collection, key });
    }
    async update(database, collection, key, document, merge = true) {
        await this.sendCommand('update', { database, collection, key, document, merge });
    }
    async delete(database, collection, key) {
        await this.sendCommand('delete', { database, collection, key });
    }
    async list(database, collection, limit = 50, offset = 0) {
        return (await this.sendCommand('list', { database, collection, limit, offset })) || [];
    }
    // Query
    async query(database, sdbql, bindVars = {}) {
        return (await this.sendCommand('query', { database, sdbql, bind_vars: bindVars })) || [];
    }
    async explain(database, sdbql, bindVars = {}) {
        return (await this.sendCommand('explain', { database, sdbql, bind_vars: bindVars })) || {};
    }
    // Transactions
    async beginTransaction(database, isolationLevel = 'read_committed') {
        return await this.sendCommand('begin_transaction', { database, isolation_level: isolationLevel });
    }
    async commitTransaction(txId) {
        await this.sendCommand('commit_transaction', { tx_id: txId });
    }
    async rollbackTransaction(txId) {
        await this.sendCommand('rollback_transaction', { tx_id: txId });
    }
}
exports.Client = Client;
