# app/services/api_list.sl
#
# Coerces a SoliDB list response into a uniform array of record hashes.
# Current servers return list endpoints as [{...}, {...}]; older servers
# (and some deployed instances) return each row positionally as an array,
# often [key, {object}] pairs. Views index rows by string key
# (row["_key"], row["name"], ...), so a raw array row raises "cannot index
# array with string" and 500s the whole page. Normalize every row to a hash.
#
# This is shape-tolerant rather than endpoint-specific, so it can guard any
# list view that iterates an API collection (cron, triggers, ...).

class ApiList
  static def normalize(raw)
    return [] unless type(raw) == "array"
    return raw.map do |entry| ApiList.row(entry) end
  end

  # hash -> as-is; array -> its first embedded hash (the record), with a
  # leading string attached as "_key" when the record carries no id; anything
  # else -> empty hash (a blank row beats a crash).
  static def row(entry)
    return entry if type(entry) == "hash"
    return {} unless type(entry) == "array"
    record = {}
    leading = ""
    for item in entry
      record = item if type(item) == "hash" && record.length() == 0
      leading = item if type(item) == "string" && leading.blank?
    end
    record = {} if record.nil?
    if !leading.blank? && (record["_key"] ?? "").blank? && (record["id"] ?? "").blank?
      record["_key"] = leading
    end
    return record
  end
end
