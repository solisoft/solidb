# Unit spec for the SoliDB value formatters (free functions, auto-loaded).
describe("solidb_helper") do
  describe("fmt_us") do
    test("dash for nil") do
      assert_eq(fmt_us(nil), "-")
    end

    test("microseconds under 1ms") do
      assert_eq(fmt_us(209), "209 µs")
    end

    test("milliseconds under 1s") do
      assert_eq(fmt_us(1240), "1.24 ms")
    end

    test("seconds above 1s") do
      assert_eq(fmt_us(2140000), "2.14 s")
    end
  end

  describe("fmt_ms") do
    test("dash for nil") do
      assert_eq(fmt_ms(nil), "-")
    end

    test("sub-millisecond shows microseconds") do
      assert_eq(fmt_ms(0.699755), "700 µs")
    end

    test("milliseconds stay milliseconds") do
      assert_eq(fmt_ms(250.5), "250.5 ms")
    end

    test("seconds above 1000ms") do
      assert_eq(fmt_ms(2140.0), "2.14 s")
    end
  end

  describe("doc_user_attrs") do
    test("empty hash for nil") do
      assert_eq(doc_user_attrs(nil), {})
    end

    test("strips the server-managed attributes") do
      doc = { "_id": "db:c/1", "_key": "1", "_rev": "r1",
              "_created_at": "2026-05-10T17:18:20Z", "_updated_at": "2026-05-10T17:18:32Z",
              "name": "Alice", "age": 30 }
      assert_eq(doc_user_attrs(doc), { "name": "Alice", "age": 30 })
    end

    test("keeps user fields that start with an underscore") do
      assert_eq(doc_user_attrs({ "_custom": 1, "name": "x" }), { "_custom": 1, "name": "x" })
    end
  end

  describe("fmt_iso") do
    test("dash for nil") do
      assert_eq(fmt_iso(nil), "-")
    end

    test("dash for blank") do
      assert_eq(fmt_iso(""), "-")
    end

    test("trims an RFC3339 timestamp to seconds") do
      assert_eq(fmt_iso("2026-05-10T17:18:20.009535303+00:00"), "2026-05-10 17:18:20")
    end
  end

  describe("fmt_epoch") do
    test("dash for nil") do
      assert_eq(fmt_epoch(nil), "-")
    end

    test("dash for zero") do
      assert_eq(fmt_epoch(0), "-")
    end

    test("formats unix seconds as UTC datetime") do
      assert_eq(fmt_epoch(1750000000), "2025-06-15 15:06:40")
    end
  end

  describe("fmt_epoch_ms") do
    test("dash for nil") do
      assert_eq(fmt_epoch_ms(nil), "-")
    end

    test("dash for zero") do
      assert_eq(fmt_epoch_ms(0), "-")
    end

    test("formats unix milliseconds as UTC datetime") do
      assert_eq(fmt_epoch_ms(1750000000123), "2025-06-15 15:06:40")
    end
  end

  describe("fmt_point_time") do
    test("dash for nil") do
      assert_eq(fmt_point_time(nil, "ms"), "-")
    end

    test("formats by the discovered encoding") do
      assert_eq(fmt_point_time(1750000000123, "ms"), "2025-06-15 15:06:40")
      assert_eq(fmt_point_time(1750000000, "s"), "2025-06-15 15:06:40")
      assert_eq(fmt_point_time("2025-06-15T15:06:40Z", "iso"), "2025-06-15 15:06:40")
    end

    test("tolerates values that don't match the encoding") do
      assert_eq(fmt_point_time("oops", "ms"), "oops")
      assert_eq(fmt_point_time(1750000000123.5, "ms"), "2025-06-15 15:06:40")
    end
  end

  describe("explain_expr") do
    test("empty for nil") do
      assert_eq(explain_expr(nil), "")
    end

    test("rewrites a comparison against a bind variable") do
      raw = "BinaryOp { left: FieldAccess(Variable(\"c\"), \"size\"), op: GreaterThan, right: BindVariable(\"s\") }"
      assert_eq(explain_expr(raw), "(c.size > @s)")
    end

    test("rewrites nested logical expressions") do
      raw = "BinaryOp { left: BinaryOp { left: FieldAccess(Variable(\"c\"), \"size\"), op: GreaterThan, " +
            "right: BindVariable(\"s\") }, op: And, right: BinaryOp { left: FieldAccess(Variable(\"c\"), " +
            "\"active\"), op: Equal, right: Literal(Bool(true)) } }"
      assert_eq(explain_expr(raw), "((c.size > @s) AND (c.active == true))")
    end

    test("rewrites chained field access and string literals") do
      raw = "BinaryOp { left: FieldAccess(FieldAccess(Variable(\"u\"), \"address\"), \"city\"), " +
            "op: Equal, right: Literal(String(\"Paris\")) }"
      assert_eq(explain_expr(raw), "(u.address.city == \"Paris\")")
    end

    test("rewrites function calls and int literals") do
      raw = "BinaryOp { left: FunctionCall(\"LENGTH\", [Variable(\"c\")]), op: GreaterThanOrEqual, " +
            "right: Literal(Int(3)) }"
      assert_eq(explain_expr(raw), "(LENGTH(c) >= 3)")
    end

    test("rewrites unary not") do
      raw = "UnaryOp { op: Not, operand: FieldAccess(Variable(\"c\"), \"deleted\") }"
      assert_eq(explain_expr(raw), "(NOT c.deleted)")
    end

    test("leaves unknown nodes intact") do
      assert_eq(explain_expr("Mystery(42)"), "Mystery(42)")
    end
  end
end
