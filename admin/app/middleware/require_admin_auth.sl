# Gate the whole admin UI behind a login.
#
# This app holds a SoliDB *administrator* credential server side
# (SOLIDB_USERNAME / SOLIDB_PASSWORD) and attaches it to every upstream call
# on behalf of whoever is browsing. Without a gate of its own, anyone who can
# reach the port gets the Lua REPL, user creation, and database drops -- the
# routes file used to say "access protection happens at the reverse-proxy
# level", but the repo ships no such proxy and the app binds 0.0.0.0.
#
# So: fail closed. Set ADMIN_UI_PASSWORD to require a login. If you really do
# terminate authentication in front of this app, set ADMIN_UI_ALLOW_NO_AUTH=1
# to say so explicitly; anything else is refused with an explanation rather
# than served wide open.
#
# The decision lives in AdminAuth so helpers can sit beside it -- every
# top-level def in a middleware file is registered as middleware.

# order: 10
# global_only: true

def require_admin_auth(req)
  return AdminAuth.gate(req)
end
