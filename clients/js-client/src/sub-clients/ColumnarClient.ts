import type { Client } from '../Client';

export class ColumnarClient {
    constructor(private client: Client) {}

    async create(name: string, columns: Array<{ name: string; type: string }>): Promise<any> {
        return this.client.sendCommand('create_columnar_table', {
            database: this.client.database,
            name,
            columns
        });
    }

    async list(): Promise<any[]> {
        return (await this.client.sendCommand('list_columnar_tables', {
            database: this.client.database
        })) || [];
    }

    async get(name: string): Promise<any> {
        return this.client.sendCommand('get_columnar_table', {
            database: this.client.database,
            name
        });
    }

    async delete(name: string): Promise<void> {
        await this.client.sendCommand('delete_columnar_table', {
            database: this.client.database,
            name
        });
    }

    async insert(name: string, rows: Array<Record<string, any>>): Promise<any> {
        return this.client.sendCommand('columnar_insert', {
            database: this.client.database,
            name,
            rows
        });
    }

    async query(name: string, query: string, params?: Record<string, any>): Promise<any[]> {
        return (await this.client.sendCommand('columnar_query', {
            database: this.client.database,
            name,
            query,
            params
        })) || [];
    }

    async aggregate(name: string, aggregation: Record<string, any>): Promise<any> {
        return this.client.sendCommand('columnar_aggregate', {
            database: this.client.database,
            name,
            aggregation
        });
    }

    async createIndex(
        tableName: string,
        indexName: string,
        column: string,
        indexType?: string
    ): Promise<any> {
        return this.client.sendCommand('columnar_create_index', {
            database: this.client.database,
            table_name: tableName,
            index_name: indexName,
            column,
            index_type: indexType
        });
    }

    async listIndexes(tableName: string): Promise<any[]> {
        return (await this.client.sendCommand('columnar_list_indexes', {
            database: this.client.database,
            table_name: tableName
        })) || [];
    }

    async deleteIndex(tableName: string, indexName: string): Promise<void> {
        await this.client.sendCommand('columnar_delete_index', {
            database: this.client.database,
            table_name: tableName,
            index_name: indexName
        });
    }

    async addColumn(
        tableName: string,
        columnName: string,
        columnType: string,
        defaultValue?: any
    ): Promise<void> {
        await this.client.sendCommand('columnar_add_column', {
            database: this.client.database,
            table_name: tableName,
            column_name: columnName,
            column_type: columnType,
            default_value: defaultValue
        });
    }

    async dropColumn(tableName: string, columnName: string): Promise<void> {
        await this.client.sendCommand('columnar_drop_column', {
            database: this.client.database,
            table_name: tableName,
            column_name: columnName
        });
    }

    async stats(tableName: string): Promise<any> {
        return this.client.sendCommand('columnar_stats', {
            database: this.client.database,
            table_name: tableName
        });
    }
}
