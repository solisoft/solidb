# Redirect to the connection form when no SoliDB credentials exist yet
# (no session override and no SOLIDB_USERNAME). Static assets and /health
# stay reachable so the setup page can render. The decision itself lives in
# SolidbClient.connection_gate — every top-level def in this file is
# registered as middleware, so helpers cannot live here.

# order: 20
# global_only: true

def require_solidb_connection(req)
  return SolidbClient.connection_gate(req, SolidbClient.credentials_configured())
end
