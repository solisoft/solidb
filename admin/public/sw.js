/* Service worker for the SoliDB admin PWA.
 *
 * Deliberately conservative: page HTML is live data behind auth and is NEVER
 * cached — navigations go to the network and only fall back to a static
 * offline page. Same-origin static assets (css/js/icons) use
 * stale-while-revalidate keyed by full URL, so the ?v= fingerprint that
 * public_path() appends naturally busts the cache on deploys.
 */
var CACHE = "solidb-admin-v2";
var OFFLINE_URL = "/offline.html";

self.addEventListener("install", function (event) {
  event.waitUntil(
    caches.open(CACHE).then(function (cache) {
      return cache.addAll([OFFLINE_URL, "/icons/icon-192.png"]);
    }).then(function () { return self.skipWaiting(); })
  );
});

self.addEventListener("activate", function (event) {
  event.waitUntil(
    caches.keys().then(function (keys) {
      return Promise.all(keys.filter(function (key) {
        return key !== CACHE;
      }).map(function (key) { return caches.delete(key); }));
    }).then(function () { return self.clients.claim(); })
  );
});

function isStaticAsset(url) {
  return url.origin === self.location.origin &&
    (url.pathname.startsWith("/css/") ||
     url.pathname.startsWith("/js/") ||
     url.pathname.startsWith("/icons/") ||
     url.pathname === "/manifest.json");
}

self.addEventListener("fetch", function (event) {
  var request = event.request;
  if (request.method !== "GET") return;

  // Navigations: network only, offline fallback. Never serve stale pages.
  if (request.mode === "navigate") {
    event.respondWith(
      fetch(request).catch(function () {
        return caches.match(OFFLINE_URL);
      })
    );
    return;
  }

  var url = new URL(request.url);
  if (!isStaticAsset(url)) return; // API/CDN/everything else: straight through

  // Stale-while-revalidate for fingerprinted static assets.
  event.respondWith(
    caches.open(CACHE).then(function (cache) {
      return cache.match(request).then(function (cached) {
        var refresh = fetch(request).then(function (response) {
          if (response && response.ok) cache.put(request, response.clone());
          return response;
        }).catch(function () { return cached; });
        return cached || refresh;
      });
    })
  );
});
