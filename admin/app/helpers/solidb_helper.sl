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

def json_compact(value)
  return "" if value.nil?
  compact = JSON.stringify(value) rescue ""
  return compact ?? ""
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

