"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.ApiKeysClient = void 0;
class ApiKeysClient {
    constructor(client) {
        this.client = client;
    }
    async list() {
        return (await this.client.sendCommand('list_api_keys', {})) || [];
    }
    async create(name, permissions, expiresAt) {
        return this.client.sendCommand('create_api_key', {
            name,
            permissions,
            expires_at: expiresAt
        });
    }
    async get(keyId) {
        return this.client.sendCommand('get_api_key', {
            key_id: keyId
        });
    }
    async delete(keyId) {
        await this.client.sendCommand('delete_api_key', {
            key_id: keyId
        });
    }
    async regenerate(keyId) {
        return this.client.sendCommand('regenerate_api_key', {
            key_id: keyId
        });
    }
}
exports.ApiKeysClient = ApiKeysClient;
