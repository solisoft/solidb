const CACHE_NAME = "talks-v14";
const STATIC_ASSETS = [
  "/favicon.png",
  "/manifest.json",
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
  self.clients.claim();
});

// Fetch event - simplified: mostly pass through, only cache manifest/icons
self.addEventListener("fetch", (event) => {
  // Skip non-GET, WebSocket, range, video, audio requests entirely
  if (event.request.method !== "GET" ||
    event.request.url.includes("/ws/") ||
    event.request.headers.has('range') ||
    event.request.destination === 'video' ||
    event.request.destination === 'audio') {
    return;
  }

  // For navigation, just go to network
  if (event.request.mode === "navigate") {
    event.respondWith(
      fetch(event.request).catch(() => caches.match("/talks"))
    );
    return;
  }

  // For everything else, network first with simple cache fallback
  event.respondWith(
    fetch(event.request)
      .then((response) => {
        // Only cache our static assets, clone BEFORE consuming
        const url = new URL(event.request.url);
        if (STATIC_ASSETS.includes(url.pathname)) {
          const clonedResponse = response.clone();
          caches.open(CACHE_NAME).then((cache) => {
            cache.put(event.request, clonedResponse);
          });
        }
        return response;
      })
      .catch(() => caches.match(event.request))
  );
});
