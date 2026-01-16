import type { Client } from '../Client';

export class GeoClient {
    constructor(private client: Client) {}

    async createIndex(
        collection: string,
        name: string,
        fields: string[],
        geoJson?: boolean
    ): Promise<any> {
        return this.client.sendCommand('create_geo_index', {
            database: this.client.database,
            collection,
            name,
            fields,
            geo_json: geoJson
        });
    }

    async listIndexes(collection: string): Promise<any[]> {
        return (await this.client.sendCommand('list_geo_indexes', {
            database: this.client.database,
            collection
        })) || [];
    }

    async deleteIndex(collection: string, indexName: string): Promise<void> {
        await this.client.sendCommand('delete_geo_index', {
            database: this.client.database,
            collection,
            index_name: indexName
        });
    }

    async near(
        collection: string,
        latitude: number,
        longitude: number,
        radius: number,
        limit?: number
    ): Promise<any[]> {
        return (await this.client.sendCommand('geo_near', {
            database: this.client.database,
            collection,
            latitude,
            longitude,
            radius,
            limit
        })) || [];
    }

    async within(collection: string, geometry: Record<string, any>): Promise<any[]> {
        return (await this.client.sendCommand('geo_within', {
            database: this.client.database,
            collection,
            geometry
        })) || [];
    }

    async distance(lat1: number, lon1: number, lat2: number, lon2: number): Promise<number> {
        return await this.client.sendCommand('geo_distance', {
            lat1,
            lon1,
            lat2,
            lon2
        });
    }

    async intersects(collection: string, geometry: Record<string, any>): Promise<any[]> {
        return (await this.client.sendCommand('geo_intersects', {
            database: this.client.database,
            collection,
            geometry
        })) || [];
    }
}
