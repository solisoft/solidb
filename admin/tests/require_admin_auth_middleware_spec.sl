# The admin UI gate.
#
# This app signs in to SoliDB as an administrator and acts under that identity
# for whoever is browsing, so an unauthenticated port here is an
# unauthenticated database. The gate must fail closed by default.
#
# `.env.test` sets ADMIN_UI_ALLOW_NO_AUTH=1 so the rest of the suite can drive
# the controllers directly; the other branches are exercised through
# `AdminAuth.gate`, which is pure and needs no process-wide state.
describe("require_admin_auth") do
  before_each() do
    as_guest()
  end

  test("lets the suite through because the test env opts out explicitly") do
    result = require_admin_auth({ "path": "/" })
    assert(result["continue"])
    assert_eq(res_status(get("/")), 200)
  end

  test("serves the login page and health probe without a session") do
    login_response = get("/login")
    assert_eq(res_status(login_response), 200)
    assert_contains(res_body(login_response), "admin-password")
    assert_eq(res_status(get("/health")), 200)
  end

  # The nav partial carries an `hx-trigger="load"` request. Rendered on the
  # sign-in page it fires without a session, gets 401 + HX-Redirect: /login,
  # and htmx navigates to /login -- which renders the nav again. The sign-in
  # page refreshed forever until the chrome was dropped from it.
  test("the sign-in page renders no app chrome and no htmx triggers") do
    body = res_body(get("/login"))
    assert_contains(body, "admin-password")
    assert_not(body.includes?("hx-trigger"))
    assert_not(body.includes?("Open navigation"))
    assert_not(body.includes?("db_picker"))
  end

  test("treats the login page, health probe and assets as public") do
    for allowed in ["/health", "/login", "/logout", "/css/application.css",
                    "/js/admin-json.js", "/manifest.json", "/sw.js"]
      assert(AdminAuth.public_path?(allowed))
    end
  end

  test("does not treat application pages as public") do
    for guarded in ["/", "/databases", "/users", "/api-keys",
                    "/databases/app/repl", "/connection"]
      assert_not(AdminAuth.public_path?(guarded))
    end
  end

  test("rejects a password that does not match") do
    assert_not(AdminAuth.password_matches?(""))
    assert_not(AdminAuth.password_matches?("anything"))
  end

  test("reads the explicit no-auth opt-out from the environment") do
    assert(AdminAuth.unauthenticated_allowed?())
  end

  test("the session key is stable so a login survives a redirect") do
    assert_eq(AdminAuth.session_key(), "admin_ui_authenticated")
  end

  # A plain 302 is only correct for a navigation. htmx and the dev
  # live-reload client both follow redirects and put the response in the DOM,
  # so redirecting them served the whole sign-in page into a fragment target
  # -- the sidebar filled with stacked sign-in forms instead of the app
  # redirecting to /login.
  test("turns scripted requests away with a status, not a redirect") do
    htmx = AdminAuth.unauthenticated_response({ "headers": { "hx-request": "true" } })
    assert_not(htmx["continue"])
    assert_eq(htmx["response"]["status"], 401)
    assert_eq(htmx["response"]["headers"]["HX-Redirect"], "/login")

    live_reload = AdminAuth.unauthenticated_response({ "headers": { "x-live-reload": "true" } })
    assert_eq(live_reload["response"]["status"], 401)

    for mode in ["cors", "no-cors", "same-origin"]
      scripted = AdminAuth.unauthenticated_response({ "headers": { "sec-fetch-mode": mode } })
      assert_eq(scripted["response"]["status"], 401)
    end

    xhr = AdminAuth.unauthenticated_response({ "headers": { "x-requested-with": "XMLHttpRequest" } })
    assert_eq(xhr["response"]["status"], 401)
  end

  test("still redirects a real page navigation to the login form") do
    for headers in [{ "sec-fetch-mode": "navigate" }, {}]
      nav = AdminAuth.unauthenticated_response({ "headers": headers })
      assert_not(nav["continue"])
      assert_eq(nav["response"]["status"], 302)
      assert_eq(nav["response"]["headers"]["Location"], "/login")
    end
  end
end
