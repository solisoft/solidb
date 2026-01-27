"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.ColumnarClient = void 0;
class ColumnarClient {
    constructor(client) {
        this.client = client;
    }
    async create(name, columns) {
        return this.client.sendCommand('create_columnar_table', {
            database: this.client.database,
            name,
            columns
        });
    }
    async list() {
        return (await this.client.sendCommand('list_columnar_tables', {
            database: this.client.database
        })) || [];
    }
    async get(name) {
        return this.client.sendCommand('get_columnar_table', {
            database: this.client.database,
            name
        });
    }
    async delete(name) {
        await this.client.sendCommand('delete_columnar_table', {
            database: this.client.database,
            name
        });
    }
    async insert(name, rows) {
        return this.client.sendCommand('columnar_insert', {
            database: this.client.database,
            name,
            rows
        });
    }
    async query(name, query, params) {
        return (await this.client.sendCommand('columnar_query', {
            database: this.client.database,
            name,
            query,
            params
        })) || [];
    }
    async aggregate(name, aggregation) {
        return this.client.sendCommand('columnar_aggregate', {
            database: this.client.database,
            name,
            aggregation
        });
    }
    async createIndex(tableName, indexName, column, indexType) {
        return this.client.sendCommand('columnar_create_index', {
            database: this.client.database,
            table_name: tableName,
            index_name: indexName,
            column,
            index_type: indexType
        });
    }
    async listIndexes(tableName) {
        return (await this.client.sendCommand('columnar_list_indexes', {
            database: this.client.database,
            table_name: tableName
        })) || [];
    }
    async deleteIndex(tableName, indexName) {
        await this.client.sendCommand('columnar_delete_index', {
            database: this.client.database,
            table_name: tableName,
            index_name: indexName
        });
    }
    async addColumn(tableName, columnName, columnType, defaultValue) {
        await this.client.sendCommand('columnar_add_column', {
            database: this.client.database,
            table_name: tableName,
            column_name: columnName,
            column_type: columnType,
            default_value: defaultValue
        });
    }
    async dropColumn(tableName, columnName) {
        await this.client.sendCommand('columnar_drop_column', {
            database: this.client.database,
            table_name: tableName,
            column_name: columnName
        });
    }
    async stats(tableName) {
        return this.client.sendCommand('columnar_stats', {
            database: this.client.database,
            table_name: tableName
        });
    }
}
exports.ColumnarClient = ColumnarClient;
