# app/services/admin_auth.sl
#
# Access control for the admin UI itself, as distinct from the SoliDB
# credential the app uses upstream.
#
# The app authenticates to SoliDB as an administrator and proxies every
# visitor's actions under that identity. Anyone who reaches the port therefore
# gets database-admin rights: the Lua REPL (`/databases/:db/repl/eval`) is
# arbitrary code execution, `/users` mints SoliDB accounts, and
# `DELETE /databases/:db` drops data. The app binds 0.0.0.0 and the repo ships
# no reverse proxy, so "protected by the proxy" was an assumption, not a
# control.
#
# Two supported configurations, and nothing else serves traffic:
#
#   ADMIN_UI_PASSWORD=<secret>     require a login here (recommended)
#   ADMIN_UI_ALLOW_NO_AUTH=1       authentication is genuinely terminated in
#                                  front of this app, and you are saying so
#
# Passwords are compared with `secure_compare` so a wrong guess costs the same
# time whatever prefix it shares with the real one.

class AdminAuth
  # Session key set once a visitor has logged in.
  static def session_key()
    return "admin_ui_authenticated"
  end

  # The configured UI password, or "" when none is set.
  static def configured_password()
    return (getenv("ADMIN_UI_PASSWORD") ?? "").trim()
  end

  static def password_configured?()
    return !AdminAuth.configured_password().blank?
  end

  # True when the operator has explicitly declared that something in front of
  # this app authenticates requests.
  static def unauthenticated_allowed?()
    flag = (getenv("ADMIN_UI_ALLOW_NO_AUTH") ?? "").trim().downcase()
    return ["1", "true", "yes"].includes?(flag)
  end

  static def logged_in?()
    return (session_get(AdminAuth.session_key()) rescue nil) == true
  end

  # Paths that must stay reachable for the login page to render and for
  # health checks to work.
  static def public_path?(request_path)
    return true if request_path == "/health"
    return true if request_path == "/login"
    return true if request_path == "/logout"
    return true if request_path == "/manifest.json"
    return true if request_path == "/sw.js"
    return true if request_path == "/offline.html"
    return true if request_path.starts_with("/css/")
    return true if request_path.starts_with("/js/")
    return true if request_path.starts_with("/icons/")
    return true if request_path.starts_with("/screenshots/")
    return false
  end

  # Verify a submitted password in constant time.
  static def password_matches?(candidate)
    expected = AdminAuth.configured_password()
    return false if expected.blank?
    return secure_compare(expected, candidate ?? "")
  end

  # Middleware decision.
  static def gate(req)
    request_path = req["path"] ?? ""
    return { "continue": true, "request": req } if AdminAuth.public_path?(request_path)

    if AdminAuth.password_configured?()
      return { "continue": true, "request": req } if AdminAuth.logged_in?()
      return AdminAuth.unauthenticated_response(req)
    end

    return { "continue": true, "request": req } if AdminAuth.unauthenticated_allowed?()

    # Neither configured: refuse rather than expose an unauthenticated
    # database administrator to the network.
    return {
      "continue": false,
      "response": {
        "status": 503,
        "headers": { "Content-Type": "text/html; charset=utf-8" },
        "body": AdminAuth.setup_required_page()
      }
    }
  end

  # How to turn away a request that has no session.
  #
  # A plain 302 to /login is only right for a *navigation*. This app drives
  # most of its UI with htmx, and `soli serve --dev` injects a live-reload
  # client that re-fetches the current URL -- both follow redirects and then
  # put the response **into the DOM**. Logged out, every one of those got the
  # whole sign-in page and swapped it into a fragment target, which is why the
  # sidebar filled up with stacked sign-in forms instead of the app
  # redirecting.
  #
  # So: only a navigation gets a redirect. Everything else gets a status the
  # caller can act on.
  static def unauthenticated_response(req)
    headers = req["headers"] ?? {}

    # htmx understands HX-Redirect: it performs a full-page navigation rather
    # than swapping the body in.
    unless (headers["hx-request"] ?? "").blank?
      return {
        "continue": false,
        "response": {
          "status": 401,
          "headers": { "HX-Redirect": "/login", "Content-Type": "text/plain" },
          "body": "unauthenticated"
        }
      }
    end

    # The dev live-reload client hard-reloads on any non-ok response, which is
    # exactly right here: the reload is a navigation and lands on /login.
    unless (headers["x-live-reload"] ?? "").blank?
      return {
        "continue": false,
        "response": {
          "status": 401,
          "headers": { "Content-Type": "text/plain" },
          "body": "unauthenticated"
        }
      }
    end

    # Any other scripted request (fetch/XHR). `Sec-Fetch-Mode: navigate` is
    # what a real page load sends; cors/no-cors mean something is reading the
    # response in JavaScript.
    fetch_mode = headers["sec-fetch-mode"] ?? ""
    requested_with = headers["x-requested-with"] ?? ""
    if ["cors", "no-cors", "same-origin"].includes?(fetch_mode) || !requested_with.blank?
      return {
        "continue": false,
        "response": {
          "status": 401,
          "headers": { "Content-Type": "text/plain" },
          "body": "unauthenticated"
        }
      }
    end

    return {
      "continue": false,
      "response": {
        "status": 302,
        "headers": { "Location": "/login" },
        "body": ""
      }
    }
  end

  static def setup_required_page()
    return "<!doctype html><meta charset=\"utf-8\">" +
           "<title>Admin UI not configured</title>" +
           "<style>body{font:15px/1.6 system-ui,sans-serif;margin:3rem auto;max-width:42rem;" +
           "padding:0 1.25rem;color:#18181b;background:#fafafa}" +
           "code{background:#e4e4e7;padding:.1em .35em;border-radius:3px}" +
           "h1{font-size:1.35rem}</style>" +
           "<h1>The admin UI has no authentication configured</h1>" +
           "<p>This application signs in to SoliDB as an administrator and " +
           "performs every visitor's actions under that identity, including " +
           "running Lua and dropping databases. It will not serve requests " +
           "until you say how visitors are authenticated.</p>" +
           "<p>Set one of:</p>" +
           "<ul>" +
           "<li><code>ADMIN_UI_PASSWORD=&lt;secret&gt;</code> — require a " +
           "login on this app.</li>" +
           "<li><code>ADMIN_UI_ALLOW_NO_AUTH=1</code> — you already " +
           "authenticate requests in front of this app and accept that this " +
           "port is unauthenticated.</li>" +
           "</ul>"
  end
end
