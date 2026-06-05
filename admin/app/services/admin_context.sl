# app/services/admin_context.sl
#
# Small shared lookups used by several controllers (e.g. the topbar database
# picker on db-scoped pages).

class AdminContext
  # Sorted database names, [] when SoliDB is unreachable.
  static def database_names()
    result = SolidbClient.get_api(SolidbEndpoints.databases())
    return [] unless result["ok"]
    names = (result["data"] ?? {})["databases"] ?? []
    return names.sort()
  end
end
