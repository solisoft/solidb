import type { Client } from '../Client';
export declare class GeoClient {
    private client;
    constructor(client: Client);
    createIndex(collection: string, name: string, fields: string[], geoJson?: boolean): Promise<any>;
    listIndexes(collection: string): Promise<any[]>;
    deleteIndex(collection: string, indexName: string): Promise<void>;
    near(collection: string, latitude: number, longitude: number, radius: number, limit?: number): Promise<any[]>;
    within(collection: string, geometry: Record<string, any>): Promise<any[]>;
    distance(lat1: number, lon1: number, lat2: number, lon2: number): Promise<number>;
    intersects(collection: string, geometry: Record<string, any>): Promise<any[]>;
}
