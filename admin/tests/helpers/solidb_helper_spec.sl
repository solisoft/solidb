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
