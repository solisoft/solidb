import type { Client } from '../Client';

export class IndexesClient {
    constructor(private client: Client) {}

    async rebuild(collection: string, indexName: string): Promise<void> {
        await this.client.sendCommand('rebuild_index', {
            database: this.client.database,
            collection,
            index_name: indexName
        });
    }

    async rebuildAll(collection: string): Promise<void> {
        await this.client.sendCommand('rebuild_all_indexes', {
            database: this.client.database,
            collection
        });
    }

    async hybridSearch(collection: string, query: Record<string, any>): Promise<any[]> {
        return (await this.client.sendCommand('hybrid_search', {
            database: this.client.database,
            collection,
            ...query
        })) || [];
    }

    async analyze(collection: string, indexName: string): Promise<any> {
        return this.client.sendCommand('analyze_index', {
            database: this.client.database,
            collection,
            index_name: indexName
        });
    }

    async getUsageStats(collection: string): Promise<any> {
        return this.client.sendCommand('index_usage_stats', {
            database: this.client.database,
            collection
        });
    }

    // --- Foreign keys ---

    /**
     * The foreign keys declared on `collection` (`foreign_keys`) and those
     * of other collections that refer to it (`referenced_by`).
     */
    async listForeignKeys(collection: string): Promise<any> {
        return this.client.sendCommand('list_foreign_keys', {
            database: this.client.database,
            collection
        });
    }

    /**
     * Declare that `field` of `collection` holds the `_key` (with
     * `by: "id"`, the `_id`) of a document of `references`, a collection of
     * the same database, or null. `onDelete` is "restrict" (the default),
     * "cascade" or "set_null". Refused when stored documents already refer
     * to a missing parent: `checkForeignKey` lists them. Returns the key as
     * declared.
     */
    async createForeignKey(
        collection: string,
        field: string,
        references: string,
        options: { onDelete?: 'restrict' | 'cascade' | 'set_null'; by?: 'key' | 'id'; name?: string } = {}
    ): Promise<any> {
        const args: Record<string, any> = {
            database: this.client.database,
            collection,
            field,
            references
        };
        if (options.onDelete !== undefined) args.on_delete = options.onDelete;
        if (options.by !== undefined) args.by = options.by;
        if (options.name !== undefined) args.name = options.name;
        return this.client.sendCommand('create_foreign_key', args);
    }

    async deleteForeignKey(collection: string, name: string): Promise<void> {
        await this.client.sendCommand('delete_foreign_key', {
            database: this.client.database,
            collection,
            name
        });
    }

    /**
     * The documents of `collection` whose `field` names a document that is
     * not in `references`: `{ count, sample }`.
     */
    async checkForeignKey(
        collection: string,
        field: string,
        references: string,
        by?: 'key' | 'id'
    ): Promise<any> {
        const args: Record<string, any> = {
            database: this.client.database,
            collection,
            field,
            references
        };
        if (by !== undefined) args.by = by;
        return this.client.sendCommand('check_foreign_key', args);
    }

    // --- Counter fields ---

    /**
     * The counter fields of `collection`: `{ counters: [...] }`, each with
     * its `state` (`next` is the next number).
     */
    async listCounters(collection: string): Promise<any> {
        return this.client.sendCommand('list_counters', {
            database: this.client.database,
            collection
        });
    }

    /**
     * Declare that the database numbers `field` on insert: each insert gets
     * the next number, never twice the same, with no gap. An insert then
     * leaves the field out and reads it from the answer. Returns the
     * counter as declared, with its state.
     */
    async createCounter(
        collection: string,
        field: string,
        options: {
            start?: number;
            reset?: 'day' | 'week' | 'month' | 'year';
            timezone?: string;
            prefix?: string;
            suffix?: string;
            padding?: number;
            durable?: boolean;
            node?: string;
        } = {}
    ): Promise<any> {
        return this.client.sendCommand('create_counter', {
            database: this.client.database,
            collection,
            counter: { ...options, field }
        });
    }

    async deleteCounter(collection: string, field: string): Promise<void> {
        await this.client.sendCommand('delete_counter', {
            database: this.client.database,
            collection,
            field
        });
    }

    /**
     * Make `next` the number the next insert gets. Refused when a document
     * already holds it.
     */
    async setCounterNext(collection: string, field: string, next: number): Promise<any> {
        return this.client.sendCommand('set_counter_next', {
            database: this.client.database,
            collection,
            field,
            next
        });
    }

    /**
     * Hand the numbering of `field` to `node` (in a cluster, one node
     * numbers), asked of the node that numbers now. With `force`, take it
     * for the node this client is connected to, for when the node that
     * numbers is lost.
     */
    async moveCounter(collection: string, field: string, node?: string, force: boolean = false): Promise<any> {
        const args: Record<string, any> = {
            database: this.client.database,
            collection,
            field,
            force
        };
        if (node !== undefined) args.node = node;
        return this.client.sendCommand('move_counter', args);
    }
}
