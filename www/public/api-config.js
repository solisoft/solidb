/**
 * Centralized API Configuration Module
 * Manages server configuration with localStorage persistence
 */

const STORAGE_KEY = 'solidb_server_config';
const RECENT_SERVERS_KEY = 'solidb_recent_servers';
const DEFAULT_CONFIG = {
    host: 'http://localhost',
    port: '6745'
};

/**
 * Get the current server configuration from localStorage or use default
 * @returns {Object} Server configuration {host, port}
 */
export function getServerConfig() {
    try {
        const stored = localStorage.getItem(STORAGE_KEY);
        if (stored) {
            return JSON.parse(stored);
        }
    } catch (e) {
        console.warn('Failed to load server config from localStorage:', e);
    }
    return { ...DEFAULT_CONFIG };
}

/**
 * Set the server configuration and persist to localStorage
 * @param {Object} config - Server configuration {host, port}
 */
export function setServerConfig(config) {
    try {
        localStorage.setItem(STORAGE_KEY, JSON.stringify(config));
        // Dispatch custom event so components can react to server changes
        window.dispatchEvent(new CustomEvent('serverConfigChanged', { detail: config }));
    } catch (e) {
        console.error('Failed to save server config to localStorage:', e);
    }
}

/**
 * Get the full API base URL based on current server configuration
 * @returns {string} Full API URL (e.g., "http://localhost:6745/_api")
 */
export function getApiUrl() {
    const config = getServerConfig();
    return `${config.host}:${config.port}/_api`;
}

/**
 * Get the list of recent/saved servers
 * @returns {Array} Array of server configs with labels
 */
export function getRecentServers() {
    try {
        const stored = localStorage.getItem(RECENT_SERVERS_KEY);
        if (stored) {
            return JSON.parse(stored);
        }
    } catch (e) {
        console.warn('Failed to load recent servers from localStorage:', e);
    }
    return [
        { host: 'http://localhost', port: '6745', label: 'Local Server' }
    ];
}

/**
 * Add a server to the recent servers list
 * @param {Object} config - Server configuration {host, port, label}
 */
export function addRecentServer(config) {
    try {
        let servers = getRecentServers();

        // Check if server already exists
        const exists = servers.find(s => s.host === config.host && s.port === config.port);
        if (exists) {
            // Update label if provided
            if (config.label) {
                exists.label = config.label;
            }
        } else {
            // Add new server
            servers.push(config);
        }

        // Keep only last 10 servers
        if (servers.length > 10) {
            servers = servers.slice(-10);
        }

        localStorage.setItem(RECENT_SERVERS_KEY, JSON.stringify(servers));
    } catch (e) {
        console.error('Failed to save recent server to localStorage:', e);
    }
}

/**
 * Remove a server from the recent servers list
 * @param {Object} config - Server configuration {host, port}
 */
export function removeRecentServer(config) {
    try {
        let servers = getRecentServers();
        servers = servers.filter(s => !(s.host === config.host && s.port === config.port));
        localStorage.setItem(RECENT_SERVERS_KEY, JSON.stringify(servers));
    } catch (e) {
        console.error('Failed to remove recent server from localStorage:', e);
    }
}

/**
 * Test connection to a server
 * @param {Object} config - Server configuration {host, port}
 * @returns {Promise<boolean>} True if connection successful
 */
export async function testConnection(config) {
    try {
        const url = `${config.host}:${config.port}/_api/databases`;
        const response = await fetch(url, {
            method: 'GET',
            signal: AbortSignal.timeout(5000) // 5 second timeout
        });
        return response.ok;
    } catch (e) {
        console.warn('Connection test failed:', e);
        return false;
    }
}
