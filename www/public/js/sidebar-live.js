/**
 * Sidebar LiveQuery Manager
 * Handles real-time updates for the activity sidebar
 */

(function () {
    const DB_HOST = window.location.host.replace(/:\d+$/, '') + ':6745';
    const WS_PROTOCOL = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
    // Get DB Name from global config or default
    const DB_NAME = (window.SoliDBConfig && window.SoliDBConfig.dbName) || 'solidb';

    let tokenCache = { token: null, expiry: 0 };
    let sidebarWs = null;
    let pingInterval;

    // Custom event dispatcher
    function notify(eventName) {
        console.log('[Sidebar] Dispatching event:', eventName);
        document.body.dispatchEvent(new CustomEvent(eventName));
    }

    // Get LiveQuery token
    async function getToken() {
        if (tokenCache.token && Date.now() < tokenCache.expiry - 5000) {
            return tokenCache.token;
        }
        try {
            const res = await fetch('/talks/livequery_token');
            if (!res.ok) throw new Error('Failed to fetch token');
            const data = await res.json();
            tokenCache.token = data.token;
            tokenCache.expiry = Date.now() + (data.expires_in || 30) * 1000;
            return data.token;
        } catch (e) {
            console.warn('[Sidebar] Token fetch failed:', e);
            return null;
        }
    }

    async function connectSidebar(retryCount = 0) {
        const token = await getToken();
        if (!token) {
            const delay = Math.min(5000 * (retryCount + 1), 30000);
            setTimeout(() => connectSidebar(retryCount + 1), delay);
            return;
        }

        const url = `${WS_PROTOCOL}//${DB_HOST}/_api/ws/changefeed?token=${token}`;
        console.log(`[Sidebar] Connecting...`);

        sidebarWs = new WebSocket(url);

        sidebarWs.onopen = () => {
            console.log(`[Sidebar] Connected`);

            // Start Ping
            if (pingInterval) clearInterval(pingInterval);
            pingInterval = setInterval(() => {
                if (sidebarWs.readyState === WebSocket.OPEN) {
                    sidebarWs.send(JSON.stringify({ type: 'ping' }));
                }
            }, 30000);

            // Send Queries with IDs
            // Tasks
            sidebarWs.send(JSON.stringify({
                type: 'live_query',
                id: 'tasks',
                database: DB_NAME,
                query: 'FOR t IN tasks RETURN t'
            }));

            // Merge Requests
            sidebarWs.send(JSON.stringify({
                type: 'live_query',
                id: 'mrs',
                database: DB_NAME,
                query: 'FOR mr IN merge_requests FILTER mr.status == "open" RETURN mr'
            }));

            // Messages
            sidebarWs.send(JSON.stringify({
                type: 'live_query',
                id: 'messages',
                database: DB_NAME,
                query: 'FOR m IN messages RETURN m'
            }));
        };

        sidebarWs.onmessage = (e) => {
            try {
                const msg = JSON.parse(e.data);
                // Dispatch based on ID
                if (msg.id === 'tasks') {
                    notify('sidebar:tasks');
                } else if (msg.id === 'mrs') {
                    notify('sidebar:mrs');
                } else if (msg.id === 'messages') {
                    notify('sidebar:messages');
                }
            } catch (err) {
                console.error(`[Sidebar] Message error:`, err);
            }
        };

        sidebarWs.onclose = () => {
            console.log(`[Sidebar] Disconnected, reconnecting in 5s...`);
            if (pingInterval) clearInterval(pingInterval);
            setTimeout(() => connectSidebar(0), 5000);
        };

        sidebarWs.onerror = (err) => {
            console.error(`[Sidebar] WebSocket error:`, err);
        };
    }

    // Start connection when DOM is ready
    if (document.readyState === 'loading') {
        document.addEventListener('DOMContentLoaded', () => connectSidebar(0));
    } else {
        connectSidebar(0);
    }

})();
