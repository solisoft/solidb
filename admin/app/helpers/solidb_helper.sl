# View formatters for SoliDB admin values (bytes, uptime, JSON blobs).

def fmt_bytes(num_bytes)
  return "0 B" if num_bytes.nil? || num_bytes <= 0
  return str(num_bytes) + " B" if num_bytes < 1024
  kb = num_bytes / 1024.0
  return str((kb * 10.0).round() / 10.0) + " KB" if kb < 1024
  mb = kb / 1024.0
  return str((mb * 10.0).round() / 10.0) + " MB" if mb < 1024
  gb = mb / 1024.0
  return str((gb * 10.0).round() / 10.0) + " GB"
end

def fmt_uptime(seconds)
  return "-" if seconds.nil? || seconds < 0
  return str(seconds) + "s" if seconds < 60
  minutes = seconds / 60
  return str(minutes) + "m" if minutes < 60
  hours = minutes / 60
  return str(hours) + "h " + str(minutes % 60) + "m" if hours < 24
  days = hours / 24
  return str(days) + "d " + str(hours % 24) + "h"
end

def fmt_percent(value)
  return "-" if value.nil?
  return str((value * 10.0).round() / 10.0) + "%"
end

# Compact large counts for dashboard cards: 1385029 -> "1.39M".
def fmt_count(value)
  return "0" if value.nil?
  return str(value) if value < 1000
  thousands = value / 1000.0
  return str((thousands * 100.0).round() / 100.0) + "k" if thousands < 1000
  millions = thousands / 1000.0
  return str((millions * 100.0).round() / 100.0) + "M" if millions < 1000
  billions = millions / 1000.0
  return str((billions * 100.0).round() / 100.0) + "B"
end

# Header connection chip. Helpers run in template scope where service
# classes (SolidbClient) are NOT visible, so the session/env fallback logic
# is mirrored here instead of delegated.
def connection_overridden
  session_host = session_get("solidb_host") rescue nil
  return !session_host.blank?
end

# Current SoliDB host without the scheme, for the header connection chip.
def connection_host_label
  full_host = session_get("solidb_host") rescue nil
  full_host = (getenv("SOLIDB_HOST") rescue nil) if full_host.blank?
  full_host = full_host ?? "http://localhost:6745"
  return full_host.substring(8, full_host.length()) if full_host.starts_with("https://")
  return full_host.substring(7, full_host.length()) if full_host.starts_with("http://")
  return full_host
end

# Heroicons-outline path (24x24 stroke) per collection kind — rendered in the
# creation modal's type picker and the collection list badges. Unknown kinds
# fall back to the document icon.
def collection_kind_icon(kind)
  icons = {
    "document": "M19.5 14.25v-2.625a3.375 3.375 0 0 0-3.375-3.375h-1.5A1.125 1.125 0 0 1 13.5 7.125v-1.5a3.375 " +
                "3.375 0 0 0-3.375-3.375H8.25m2.25 0H5.625c-.621 0-1.125.504-1.125 1.125v17.25c0 .621.504 1.125 " +
                "1.125 1.125h12.75c.621 0 1.125-.504 1.125-1.125V11.25a9 9 0 0 0-9-9Z",
    "edge": "M7.217 10.907a2.25 2.25 0 1 0 0 2.186m0-2.186c.18.324.283.696.283 1.093s-.103.77-.283 " +
            "1.093m0-2.186 9.566-5.314m-9.566 7.5 9.566 5.314m0 0a2.25 2.25 0 1 0 3.935 2.186 2.25 2.25 0 0 " +
            "0-3.935-2.186Zm0-12.814a2.25 2.25 0 1 0 3.933-2.185 2.25 2.25 0 0 0-3.933 2.185Z",
    "blob": "m18.375 12.739-7.693 7.693a4.5 4.5 0 0 1-6.364-6.364l10.94-10.94A3 3 0 1 1 19.5 " +
            "7.372L8.552 18.32m.009-.01-.01.01m5.699-9.941-7.81 7.81a1.5 1.5 0 0 0 2.112 2.13",
    "timeseries": "M2.25 18 9 11.25l4.306 4.306a11.95 11.95 0 0 1 5.814-5.518l2.74-1.22m0 " +
                  "0-5.94-2.281m5.94 2.28-2.28 5.941",
    "columnar": "M3 13.125C3 12.504 3.504 12 4.125 12h2.25c.621 0 1.125.504 1.125 1.125v6.75C7.5 20.496 " +
                "6.996 21 6.375 21h-2.25A1.125 1.125 0 0 1 3 19.875v-6.75ZM9.75 8.625c0-.621.504-1.125 " +
                "1.125-1.125h2.25c.621 0 1.125.504 1.125 1.125v11.25c0 .621-.504 1.125-1.125 1.125h-2.25a1.125 " +
                "1.125 0 0 1-1.125-1.125V8.625ZM16.5 4.125c0-.621.504-1.125 1.125-1.125h2.25C20.496 3 21 3.504 " +
                "21 4.125v15.75c0 .621-.504 1.125-1.125 1.125h-2.25a1.125 1.125 0 0 1-1.125-1.125V4.125Z"
  }
  return icons[kind] ?? icons["document"]
end

# Server-managed attributes stripped from a document before editing: they are
# read-only (the PUT identifies the doc by URL key; _rev CAS goes via If-Match).
def doc_user_attrs(doc)
  return {} if doc.nil?
  system_attrs = ["_id", "_key", "_rev", "_created_at", "_updated_at"]
  user_doc = {}
  for field in doc.keys()
    user_doc[field] = doc[field] unless system_attrs.includes?(field)
  end
  return user_doc
end

# RFC3339 string ("2026-05-10T17:18:20.009535303+00:00") -> "2026-05-10 17:18:20".
def fmt_iso(timestamp)
  return "-" if timestamp.nil? || timestamp.blank?
  return str(timestamp).substring(0, 19).replace("T", " ")
end

# Unix seconds -> "YYYY-MM-DD HH:MM:SS" (UTC). "-" when missing/zero.
def fmt_epoch(seconds)
  return "-" if seconds.nil? || seconds <= 0
  iso = DateTime.from_unix(seconds).to_iso() rescue ""
  return str(seconds) if iso.blank?
  return iso.substring(0, 19).replace("T", " ")
end

# Unix milliseconds -> same shape (job started_at/completed_at are in ms).
def fmt_epoch_ms(milliseconds)
  return "-" if milliseconds.nil? || milliseconds <= 0
  return fmt_epoch(milliseconds / 1000)
end

def json_compact(value)
  return "" if value.nil?
  compact = JSON.stringify(value) rescue ""
  return compact ?? ""
end

# Timeseries point timestamp -> human label. kind is the discovered encoding
# ("ms" / "s" / "iso"); tolerant of stray values that don't match it.
def fmt_point_time(value, kind)
  return "-" if value.nil?
  return fmt_iso(str(value)) if kind == "iso"
  is_number = type(value) == "int" || type(value) == "float"
  return str(value) if !is_number
  return fmt_epoch(int(value)) if kind == "s"
  return fmt_epoch_ms(int(value))
end

# Milliseconds (float, e.g. the cursor's executionTimeMs) -> µs / ms / s,
# whichever reads best. 0.699755 -> "700 µs", 801.56 -> "801.56 ms".
def fmt_ms(milliseconds)
  return "-" if milliseconds.nil?
  return fmt_us((milliseconds * 1000.0).round())
end

def fmt_us(microseconds)
  return "-" if microseconds.nil?
  return str(microseconds) + " µs" if microseconds < 1000
  milliseconds = microseconds / 1000.0
  return str((milliseconds * 100.0).round() / 100.0) + " ms" if milliseconds < 1000
  seconds = milliseconds / 1000.0
  return str((seconds * 100.0).round() / 100.0) + " s"
end

# The explain API returns filter expressions as Rust AST debug strings
# ("BinaryOp { left: FieldAccess(Variable(\"c\"), \"age\"), ... }").
# Rewrite the common nodes back into SDBQL-ish syntax; unknown nodes are
# left as-is, so a partial rewrite still beats the raw dump.
def explain_expr(raw)
  return "" if raw.nil?
  text = raw
  # Leaves. BindVariable must run before Variable (it contains it).
  text = text.gsub("BindVariable\\(\"([\\w]+)\"\\)", "@$1")
  text = text.gsub("Variable\\(\"([\\w]+)\"\\)", "$1")
  text = text.gsub("Literal\\(Bool\\((\\w+)\\)\\)", "$1")
  text = text.gsub("Literal\\(Int\\((-?\\d+)\\)\\)", "$1")
  text = text.gsub("Literal\\(Float\\((-?[\\d.]+)\\)\\)", "$1")
  text = text.gsub("Literal\\(String\\((\"[^\"]*\")\\)\\)", "$1")
  text = text.gsub("Literal\\(Null\\)", "null")
  # Composite nodes, innermost first - repeat until nothing changes.
  prev = ""
  while prev != text
    prev = text
    text = text.gsub("FieldAccess\\(([\\w.@\"\\[\\]]+), \"([\\w]+)\"\\)", "$1.$2")
    text = text.gsub("FunctionCall\\(\"([\\w]+)\", \\[([^\\[\\]{}]*)\\]\\)", "$1($2)")
    text = text.gsub("UnaryOp \\{ op: Not, operand: ([^{}]*?) \\}", "(NOT $1)")
    text = text.gsub("UnaryOp \\{ op: Negate, operand: ([^{}]*?) \\}", "(-$1)")
    text = text.gsub("BinaryOp \\{ left: ([^{}]*?), op: ([\\w]+), right: ([^{}]*?) \\}", "($1 $2 $3)")
  end
  operator_symbols = {
    " Equal ": " == ", " NotEqual ": " != ", " LessThan ": " < ",
    " LessThanOrEqual ": " <= ", " GreaterThan ": " > ", " GreaterThanOrEqual ": " >= ",
    " In ": " IN ", " NotIn ": " NOT IN ", " And ": " AND ", " Or ": " OR ",
    " Add ": " + ", " Subtract ": " - ", " Multiply ": " * ", " Divide ": " / ",
    " Modulus ": " % ", " Exponent ": " ^ ", " Like ": " LIKE ", " NotLike ": " NOT LIKE ",
    " RegEx ": " =~ ", " NotRegEx ": " !~ ", " FuzzyEqual ": " ~= "
  }
  for operator_name in operator_symbols.keys()
    text = text.replace(operator_name, operator_symbols[operator_name])
  end
  return text
end

