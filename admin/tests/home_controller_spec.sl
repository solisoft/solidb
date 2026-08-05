describe("HomeController") do
  before_each() do
    as_guest()
  end

  describe("GET /") do
    test("renders the dashboard with server status and node info") do
      response = get("/")
      assert_eq(res_status(response), 200)
      body = res_body(response)
      assert_contains(body, "Dashboard")
      assert_contains(body, "ONLINE")
      assert_contains(body, "node id")
      assert_contains(body, "Databases")
    end
  end

  describe("GET /health") do
    test("returns ok JSON") do
      response = get("/health")
      assert_eq(res_status(response), 200)
      assert_eq(res_json(response)["status"], "ok")
    end
  end

  describe("PWA") do
    test("layout wires up the manifest and the service worker") do
      response = get("/")
      body = res_body(response)
      assert_contains(body, "rel=\"manifest\"")
      assert_contains(body, "/manifest.json")
      assert_contains(body, "theme-color")
      assert_contains(body, "apple-touch-icon")
      assert_contains(body, "serviceWorker")
      assert_contains(body, "navigator.serviceWorker.register(\"/sw.js\")")
    end

    test("serves the manifest with the install-required fields") do
      response = get("/manifest.json")
      assert_eq(res_status(response), 200)
      manifest = res_json(response)
      assert_eq(manifest["name"], "SoliDB Admin")
      assert_eq(manifest["display"], "standalone")
      assert_eq(manifest["start_url"], "/")
      icon_sizes = manifest["icons"].map do |icon| icon["sizes"] end
      assert_contains(icon_sizes, "192x192")
      assert_contains(icon_sizes, "512x512")
      # Richer install UI needs one wide + one narrow screenshot.
      form_factors = manifest["screenshots"].map do |shot| shot["form_factor"] end
      assert_contains(form_factors, "wide")
      assert_contains(form_factors, "narrow")
    end

    test("serves the service worker, icons and offline fallback") do
      for asset in ["/sw.js", "/offline.html", "/icons/icon-192.png",
                    "/icons/icon-512.png", "/icons/maskable-192.png", "/icons/maskable-512.png",
                    "/screenshots/wide.png", "/screenshots/narrow.png"]
        response = get(asset)
        assert_eq(res_status(response), 200)
      end
      response = get("/sw.js")
      assert_contains(res_body(response), "stale-while-revalidate")
      assert_contains(res_body(response), "offline.html")
    end

    test("service worker retries a navigation before falling back to the offline page") do
      body = res_body(get("/sw.js"))
      # Bumping the cache name is what makes clients pick up a new offline page.
      assert_contains(body, "solidb-admin-v3")
      assert_contains(body, "function navigate(request)")
      # No cached fallback: surface the browser's real network error instead.
      assert_contains(body, "Response.error()")
    end

    test("offline page names the unreachable host and recovers on its own") do
      body = res_body(get("/offline.html"))
      assert_contains(body, "location.origin")
      assert_contains(body, "navigator.onLine")
      assert_contains(body, "/health?_pwa_probe=")
      assert_contains(body, "location.reload()")
    end
  end
end
