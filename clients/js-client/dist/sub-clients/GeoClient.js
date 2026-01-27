"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.GeoClient = void 0;
class GeoClient {
    constructor(client) {
        this.client = client;
    }
    async createIndex(collection, name, fields, geoJson) {
        return this.client.sendCommand('create_geo_index', {
            database: this.client.database,
            collection,
            name,
            fields,
            geo_json: geoJson
        });
    }
    async listIndexes(collection) {
        return (await this.client.sendCommand('list_geo_indexes', {
            database: this.client.database,
            collection
        })) || [];
    }
    async deleteIndex(collection, indexName) {
        await this.client.sendCommand('delete_geo_index', {
            database: this.client.database,
            collection,
            index_name: indexName
        });
    }
    async near(collection, latitude, longitude, radius, limit) {
        return (await this.client.sendCommand('geo_near', {
            database: this.client.database,
            collection,
            latitude,
            longitude,
            radius,
            limit
        })) || [];
    }
    async within(collection, geometry) {
        return (await this.client.sendCommand('geo_within', {
            database: this.client.database,
            collection,
            geometry
        })) || [];
    }
    async distance(lat1, lon1, lat2, lon2) {
        return await this.client.sendCommand('geo_distance', {
            lat1,
            lon1,
            lat2,
            lon2
        });
    }
    async intersects(collection, geometry) {
        return (await this.client.sendCommand('geo_intersects', {
            database: this.client.database,
            collection,
            geometry
        })) || [];
    }
}
exports.GeoClient = GeoClient;
