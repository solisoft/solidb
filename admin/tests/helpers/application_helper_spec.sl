# Unit spec for the scaffold view helpers (free functions, auto-loaded).
describe("application_helper") do
  describe("truncate_text") do
    test("returns short text unchanged") do
      assert_eq(truncate_text("hello", 10, "..."), "hello")
    end

    test("truncates long text with the suffix") do
      assert_eq(truncate_text("hello world", 8, "..."), "hello...")
    end
  end

  describe("capitalize") do
    test("uppercases the first letter") do
      assert_eq(capitalize("soli"), "Soli")
    end

    test("keeps an empty string empty") do
      assert_eq(capitalize(""), "")
    end
  end

  describe("link helpers") do
    test("link_to escapes text and keeps safe urls") do
      assert_eq(link_to("Docs <3", "/docs"), "<a href=\"/docs\">Docs &lt;3</a>")
    end

    test("link_to_class adds the css class") do
      assert_eq(link_to_class("Docs", "https://x.dev", "btn"),
                "<a href=\"https://x.dev\" class=\"btn\">Docs</a>")
    end

    test("javascript: urls are neutralized to #") do
      assert_eq(_safe_link_url("javascript:alert(1)"), "#")
      assert_eq(link_to("evil", "javascript:alert(1)"), "<a href=\"#\">evil</a>")
    end

    test("safe scheme and relative shapes are accepted") do
      assert(_is_safe_link_url("https://example.com"))
      assert(_is_safe_link_url("http://example.com"))
      assert(_is_safe_link_url("mailto:a@b.c"))
      assert(_is_safe_link_url("/path"))
      assert(_is_safe_link_url("#anchor"))
      assert(_is_safe_link_url("?q=1"))
      assert(_is_safe_link_url("relative/path"))
      assert(_is_safe_link_url("page#frag"))
      assert(_is_safe_link_url("page?q=1"))
    end

    test("custom schemes are refused") do
      assert_not(_is_safe_link_url("data:text/html,x"))
      assert_not(_is_safe_link_url("vbscript:msgbox"))
    end
  end

  describe("pluralize") do
    test("singular at exactly one") do
      assert_eq(pluralize(1, "entry", "entries"), "1 entry")
      assert_eq(pluralize(3, "entry", "entries"), "3 entries")
    end

    test("pluralize_simple appends s") do
      assert_eq(pluralize_simple(1, "doc"), "1 doc")
      assert_eq(pluralize_simple(2, "doc"), "2 docs")
    end
  end
end
