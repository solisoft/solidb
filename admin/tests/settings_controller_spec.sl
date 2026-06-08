# Settings - theme preset picker (cookie-backed).
describe("SettingsController") do
  before_each() do
    as_guest()
    clear_cookies()
  end

  describe("GET /settings") do
    test("lists every theme preset") do
      response = get("/settings")
      assert_eq(res_status(response), 200)
      body = res_body(response)
      assert_contains(body, "solidb")
      assert_contains(body, "arangodb")
      assert_contains(body, "violet")
      assert_contains(body, "amber")
      assert_contains(body, "rose")
      assert_contains(body, "sky")
      assert_contains(body, "solidb-light")
      assert_contains(body, "arangodb-light")
      assert_contains(body, "light presets")
    end

    test("marks the default preset active without a cookie") do
      response = get("/settings")
      body = res_body(response)
      solidb_card = body.split(">solidb<")[1].split("</form>")[0]
      assert_contains(solidb_card, "active")
    end

    test("marks the cookie preset active") do
      set_request_cookie("admin_theme", "arangodb")
      response = get("/settings")
      assert_eq(res_status(response), 200)
      arangodb_card = res_body(response).split(">arangodb<")[1].split("</form>")[0]
      assert_contains(arangodb_card, "active")
    end
  end

  describe("POST /settings/theme") do
    test("sets the cookie and redirects back") do
      response = post("/settings/theme", { "theme": "arangodb" })
      assert(res_redirect?(response))
      assert_contains(res_header(response, "Set-Cookie"), "admin_theme=arangodb")
      assert_contains(res_location(response), "/settings")
    end

    test("whitelists unknown presets back to solidb") do
      response = post("/settings/theme", { "theme": "\"><script>alert(1)</script>" })
      assert(res_redirect?(response))
      assert_contains(res_header(response, "Set-Cookie"), "admin_theme=solidb")
    end
  end

  describe("font presets") do
    test("lists every font preset") do
      response = get("/settings")
      body = res_body(response)
      assert_contains(body, "grotesk")
      assert_contains(body, "inter")
      assert_contains(body, "ibm-plex")
      assert_contains(body, "source")
      assert_contains(body, "system")
    end

    test("POST sets the cookie and redirects back") do
      response = post("/settings/font", { "font": "inter" })
      assert(res_redirect?(response))
      assert_contains(res_header(response, "Set-Cookie"), "admin_font=inter")
    end

    test("whitelists unknown fonts back to grotesk") do
      response = post("/settings/font", { "font": "comic-sans\"><script>" })
      assert(res_redirect?(response))
      assert_contains(res_header(response, "Set-Cookie"), "admin_font=grotesk")
    end

    test("body carries data-font and loads only that preset's fonts") do
      set_request_cookie("admin_font", "inter")
      response = get("/settings")
      body = res_body(response)
      assert_contains(body, "data-font=\"inter\"")
      assert_contains(body, "family=Inter")
      assert_not(body.includes?("family=Space+Grotesk"))
    end

    test("system fonts load no webfont at all") do
      set_request_cookie("admin_font", "system")
      response = get("/settings")
      body = res_body(response)
      assert_contains(body, "data-font=\"system\"")
      assert_not(body.includes?("fonts.googleapis.com"))
    end
  end

  describe("layout integration") do
    test("body carries the whitelisted data-theme") do
      set_request_cookie("admin_theme", "arangodb")
      response = get("/settings")
      assert_contains(res_body(response), "data-theme=\"arangodb\"")
    end

    test("light presets pass the whitelist") do
      set_request_cookie("admin_theme", "solidb-light")
      response = get("/settings")
      assert_contains(res_body(response), "data-theme=\"solidb-light\"")
    end

    test("a tampered cookie falls back to solidb") do
      set_request_cookie("admin_theme", "\"><script>")
      response = get("/settings")
      assert_contains(res_body(response), "data-theme=\"solidb\"")
      assert_not(res_body(response).includes?("data-theme=\"\"><script>"))
    end
  end
end
