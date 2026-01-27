import type { Client } from '../Client';
export declare class ColumnarClient {
    private client;
    constructor(client: Client);
    create(name: string, columns: Array<{
        name: string;
        type: string;
    }>): Promise<any>;
    list(): Promise<any[]>;
    get(name: string): Promise<any>;
    delete(name: string): Promise<void>;
    insert(name: string, rows: Array<Record<string, any>>): Promise<any>;
    query(name: string, query: string, params?: Record<string, any>): Promise<any[]>;
    aggregate(name: string, aggregation: Record<string, any>): Promise<any>;
    createIndex(tableName: string, indexName: string, column: string, indexType?: string): Promise<any>;
    listIndexes(tableName: string): Promise<any[]>;
    deleteIndex(tableName: string, indexName: string): Promise<void>;
    addColumn(tableName: string, columnName: string, columnType: string, defaultValue?: any): Promise<void>;
    dropColumn(tableName: string, columnName: string): Promise<void>;
    stats(tableName: string): Promise<any>;
}
