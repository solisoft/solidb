# Login for the admin UI itself. See AdminAuth for why this exists: the app
# holds a SoliDB administrator credential and acts under it for whoever is
# browsing, so it needs a door of its own.

class SessionController < Controller
  static {
    this.layout = "application"
  }

  # GET /login
  def new
    this._ctx()
    return redirect(databases_path()) if AdminAuth.logged_in?()
    return render("session/new")
  end

  # POST /login
  def create
    this._ctx()
    unless AdminAuth.password_configured?()
      @login_error = "no ADMIN_UI_PASSWORD is configured on this server"
      return render("session/new")
    end
    unless AdminAuth.password_matches?(params["password"] ?? "")
      # Deliberately says nothing about what was wrong.
      @login_error = "incorrect password"
      return render("session/new")
    end
    # New session id on privilege change, so a pre-login session cookie
    # cannot be fixed onto an authenticated session.
    session_regenerate
    session_set(AdminAuth.session_key(), true)
    return redirect(databases_path())
  end

  # POST /logout
  def destroy
    session_delete(AdminAuth.session_key())
    session_regenerate
    return redirect("/login")
  end

  def _ctx
    @title = "Sign in"
    @db = ""
    @databases = []
    @login_error = ""
    # No sidebar, no database picker, no connection indicator on the sign-in
    # page. Beyond looking wrong, the nav partial fires an `hx-trigger="load"`
    # request that, without a session, comes back 401 + HX-Redirect: /login —
    # htmx then navigates to /login, which renders the nav again. That is a
    # self-sustaining refresh loop on the sign-in page itself.
    @hide_chrome = true
  end
end
