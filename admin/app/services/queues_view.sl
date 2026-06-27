# app/services/queues_view.sl
#
# Adapts the SoliDB queues API response into a uniform shape for the queues
# views. Current SoliDB returns the queue list as stat objects, but older
# builds return each queue positionally as an array. The index view indexes
# queue["name"], so a raw array entry would raise "cannot index array with
# string" and 500 the whole page -- normalize every entry to a hash first.

class QueuesView
  # Always returns an array of {name, pending, running, completed, failed}
  # hashes; [] for any non-array payload (e.g. an error body).
  static def normalize(raw)
    return [] unless type(raw) == "array"
    return raw.map do |entry| QueuesView.normalize_one(entry) end
  end

  static def normalize_one(entry)
    return entry if type(entry) == "hash"
    if type(entry) != "array"
      return { "name": str(entry), "pending": 0, "running": 0, "completed": 0, "failed": 0 }
    end
    # Array entry: take the name from the first string, stats from an embedded
    # hash. Unknown positional layouts degrade to zeroed counts, never a crash.
    stats = {}
    name = ""
    for item in entry
      stats = item if type(item) == "hash" && stats.length() == 0
      name = item if type(item) == "string" && name.blank?
    end
    return {
      "name": name.blank? ? (stats["name"] ?? "") : name,
      "pending": stats["pending"] ?? 0,
      "running": stats["running"] ?? 0,
      "completed": stats["completed"] ?? 0,
      "failed": stats["failed"] ?? 0,
      # Per-queue settings (newer SoliDB). Legacy array payloads predate these,
      # so default to the unconfigured state.
      "paused": stats["paused"] ?? false,
      "concurrency": stats["concurrency"] ?? 0,
      "default_priority": stats["default_priority"] ?? 0
    }
  end
end
