const CACHE_NAME = "talks-v11";
const STATIC_ASSETS = [
  "/favicon.png",
  "/manifest.json",
  "/app.css",
  "/images/icon-192.png",
  "/images/icon-512.png"
];

// Install event - cache static assets
self.addEventListener("install", (event) => {
  event.waitUntil(
    caches.open(CACHE_NAME).then((cache) => {
      return cache.addAll(STATIC_ASSETS);
    })
  );
  // Activate immediately
  self.skipWaiting();
});

// Activate event - clean old caches
self.addEventListener("activate", (event) => {
  event.waitUntil(
    caches.keys().then((cacheNames) => {
      return Promise.all(
        cacheNames
          .filter((name) => name !== CACHE_NAME)
          .map((name) => caches.delete(name))
      );
    })
  );
  // Take control immediately
  self.clients.claim();
});

// Fetch event - network first, fallback to cache
self.addEventListener("fetch", (event) => {
  // Skip non-GET requests and WebSocket connections
  if (event.request.method !== "GET" || event.request.url.includes("/ws/")) {
    return;
  }

  // Skip video/audio requests and range requests to let browser handle them
  if (event.request.headers.has('range') ||
    event.request.destination === 'video' ||
    event.request.destination === 'audio') {
    return;
  }

  // For navigation requests, use network first
  if (event.request.mode === "navigate") {
    event.respondWith(
      fetch(event.request)
        .catch(() => caches.match("/talks"))
    );
    return;
  }

  // For static assets, try cache first then network
  event.respondWith(
    caches.match(event.request).then((cachedResponse) => {
      if (cachedResponse) {
        // Return cached and update in background
        fetch(event.request).then((response) => {
          if (response && response.status === 200) {
            const responseToCache = response.clone();
            caches.open(CACHE_NAME).then((cache) => {
              cache.put(event.request, responseToCache);
            });
          }
        }).catch(() => { });
        return cachedResponse;
      }

      // Not in cache, fetch from network
      return fetch(event.request).then((response) => {
        // Cache static assets
        if (response && response.status === 200) {
          const url = new URL(event.request.url);
          if (url.pathname.match(/\.(css|js|svg|png|jpg|jpeg|webp|woff2?)$/)) {
            caches.open(CACHE_NAME).then((cache) => {
              cache.put(event.request, response.clone());
            });
          }
        }
        return response;
      });
    })
  );
});
