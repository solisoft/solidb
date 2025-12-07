/**
 * Centralized API Configuration Module
 * Manages server configuration with localStorage persistence
 */

const STORAGE_KEY = 'solidb_server_config';
const RECENT_SERVERS_KEY = 'solidb_recent_servers';
const AUTH_TOKEN_KEY = 'solidb_auth_token';
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
 * Get the login URL for authentication
 * @returns {string} Login URL (e.g., "http://localhost:6745/auth/login")
 */
export function getLoginUrl() {
    const config = getServerConfig();
    return `${config.host}:${config.port}/auth/login`;
}

/**
 * Get the stored authentication token
 * @returns {string|null} JWT token or null if not authenticated
 */
export function getAuthToken() {
    try {
        return localStorage.getItem(AUTH_TOKEN_KEY);
    } catch (e) {
        console.warn('Failed to get auth token:', e);
        return null;
    }
}

/**
 * Set the authentication token
 * @param {string} token - JWT token
 */
export function setAuthToken(token) {
    try {
        localStorage.setItem(AUTH_TOKEN_KEY, token);
    } catch (e) {
        console.error('Failed to save auth token:', e);
    }
}

/**
 * Clear the authentication token (logout)
 */
export function clearAuthToken() {
    try {
        localStorage.removeItem(AUTH_TOKEN_KEY);
    } catch (e) {
        console.error('Failed to clear auth token:', e);
    }
}

/**
 * Check if user is authenticated
 * @returns {boolean} True if token exists
 */
export function isAuthenticated() {
    return !!getAuthToken();
}

/**
 * Redirect to login page
 */
export function redirectToLogin() {
    window.location.href = '/login';
}

/**
 * Authenticated fetch wrapper
 * Automatically includes Bearer token and handles 401 responses
 * @param {string} url - URL to fetch
 * @param {Object} options - Fetch options
 * @returns {Promise<Response>} Fetch response
 */
export async function authenticatedFetch(url, options = {}) {
    const token = getAuthToken();

    if (!token) {
        redirectToLogin();
        return Promise.reject(new Error('Not authenticated'));
    }

    const headers = {
        ...options.headers,
        'Authorization': `Bearer ${token}`
    };

    try {
        const response = await fetch(url, { ...options, headers });

        if (response.status === 401) {
            clearAuthToken();
            redirectToLogin();
            return Promise.reject(new Error('Session expired'));
        }

        return response;
    } catch (e) {
        throw e;
    }
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
        const token = getAuthToken();
        const url = `${config.host}:${config.port}/_api/databases`;
        const headers = token ? { 'Authorization': `Bearer ${token}` } : {};
        const response = await fetch(url, {
            method: 'GET',
            headers,
            signal: AbortSignal.timeout(5000) // 5 second timeout
        });
        return response.ok;
    } catch (e) {
        console.warn('Connection test failed:', e);
        return false;
    }
}
