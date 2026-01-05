-- Tests for I18n module
-- test/i18n_test.lua

package.path = package.path .. ";.lua/?.lua;config/locales/?.lua"

local Test = require("test")
local describe, it, expect, before = Test.describe, Test.it, Test.expect, Test.before

-- Load I18n module
local I18n = require("i18n")

-- Helper: table.contains (if not available)
if not table.contains then
  table.contains = function(tbl, val)
    for _, v in pairs(tbl) do
      if v == val then return true end
    end
    return false
  end
end

describe("I18n", function()

  -- Setup mock translations for testing
  local function setup()
    I18n.translations = {
      en = {
        hello = "Hello",
        welcome = "Welcome, %{name}!",
        items_count = "You have %d items",
        models = {
          errors = {
            presence = "can't be blank",
            format = "is invalid"
          }
        },
        date = {
          formats = {
            default = "%Y-%m-%d"
          }
        },
        time = {
          formats = {
            default = "%H:%M:%S"
          }
        },
        datetime = {
          formats = {
            default = "%Y-%m-%d %H:%M:%S"
          }
        }
      },
      fr = {
        hello = "Bonjour",
        welcome = "Bienvenue, %{name}!",
        items_count = "Vous avez %d éléments",
        models = {
          errors = {
            presence = "ne peut pas être vide"
          }
        }
      }
    }
    I18n.loaded_locales = { en = true, fr = true }
    I18n:set_locale("en")
  end

  describe("Translation", function()
    before(setup)

    it("should translate simple key", function()
      local result = I18n:t("hello")
      expect.eq(result, "Hello")
    end)

    it("should return key if not found", function()
      local result = I18n:t("nonexistent.key")
      expect.eq(result, "nonexistent.key")
    end)

    it("should support nested keys", function()
      local result = I18n:t("models.errors.presence")
      expect.eq(result, "can't be blank")
    end)

    it("should support string interpolation with named parameters", function()
      local result = I18n:t("welcome", { name = "John" })
      expect.eq(result, "Welcome, John!")
    end)

    it("should support string interpolation with positional parameters", function()
      local result = I18n:t("items_count", 5)
      expect.eq(result, "You have 5 items")
    end)

    it("should handle missing interpolation parameters", function()
      local result = I18n:t("welcome", { name = nil })
      expect.eq(result, "Welcome, %{name}!")
    end)

    it("should use fallback locale if translation not found", function()
      I18n:set_locale("fr")
      local result = I18n:t("models.errors.format")
      expect.eq(result, "is invalid") -- Fallback to English
    end)

    it("should translate with translate alias", function()
      local result = I18n:translate("hello")
      expect.eq(result, "Hello")
    end)
  end)

  describe("Locale management", function()
    before(setup)

    it("should set current locale", function()
      local result = I18n:set_locale("fr")
      expect.truthy(result)
      expect.eq(I18n:get_locale(), "fr")
    end)

    it("should get current locale", function()
      I18n:set_locale("en")
      expect.eq(I18n:get_locale(), "en")
    end)

    it("should return false for invalid locale", function()
      local result = I18n:set_locale("invalid")
      expect.falsy(result)
    end)

    it("should list available locales", function()
      local locales = I18n:available_locales()
      expect.truthy(#locales > 0)
      expect.truthy(table.contains(locales, "en"))
      expect.truthy(table.contains(locales, "fr"))
    end)
  end)

  describe("exists", function()
    before(setup)

    it("should return true for existing key", function()
      expect.truthy(I18n:exists("hello"))
    end)

    it("should return true for nested key", function()
      expect.truthy(I18n:exists("models.errors.presence"))
    end)

    it("should return false for non-existing key", function()
      expect.falsy(I18n:exists("nonexistent.key"))
    end)

    it("should check specific locale", function()
      expect.truthy(I18n:exists("hello", "en"))
      expect.falsy(I18n:exists("hello", "nonexistent"))
    end)
  end)

  describe("Pluralization", function()
    before(setup)

    it("should return singular for count 1", function()
      local result = I18n:p(1, "item", "items")
      expect.eq(result, "item")
    end)

    it("should return plural for count > 1", function()
      local result = I18n:p(2, "item", "items")
      expect.eq(result, "items")
    end)

    it("should use table forms for zero", function()
      local result = I18n:p(0, { zero = "No items", one = "1 item", other = "%d items" })
      expect.eq(result, "No items")
    end)

    it("should use table forms for one", function()
      local result = I18n:p(1, { zero = "No items", one = "1 item", other = "%d items" })
      expect.eq(result, "1 item")
    end)

    it("should use table forms for other", function()
      local result = I18n:p(5, { zero = "No items", one = "1 item", other = "%d items" })
      expect.eq(result, "5 items")
    end)

    it("should default to simple pluralization", function()
      local result = I18n:p(2, "item")
      expect.eq(result, "items")
    end)
  end)

  describe("Number localization", function()
    before(setup)

    it("should format simple number", function()
      local result = I18n:number(1234)
      expect.eq(result, "1,234")
    end)

    it("should format number with precision", function()
      local result = I18n:number(1234.567, { precision = 2 })
      expect.eq(result, "1,234.57")
    end)

    it("should format large number", function()
      local result = I18n:number(1000000)
      expect.eq(result, "1,000,000")
    end)

    it("should use custom delimiter", function()
      local result = I18n:number(1234, { delimiter = " " })
      expect.eq(result, "1 234")
    end)

    it("should use custom separator", function()
      local result = I18n:number(1234.56, { precision = 2, separator = "," })
      expect.eq(result, "1,234,56")
    end)
  end)

  describe("Date/Time localization", function()
    before(setup)

    it("should format date with default format", function()
      local timestamp = os.time({ year = 2025, month = 12, day = 31 })
      local result = I18n:date(timestamp)
      expect.eq(result, "2025-12-31")
    end)

    it("should format date with custom format", function()
      local timestamp = os.time({ year = 2025, month = 12, day = 31 })
      local result = I18n:date(timestamp, "%d/%m/%Y")
      expect.eq(result, "31/12/2025")
    end)

    it("should format time", function()
      local timestamp = os.time({ year = 2025, month = 12, day = 31, hour = 14, min = 30, sec = 45 })
      local result = I18n:time(timestamp)
      expect.eq(result, "14:30:45")
    end)

    it("should format datetime", function()
      local timestamp = os.time({ year = 2025, month = 12, day = 31, hour = 14, min = 30 })
      local result = I18n:datetime(timestamp, "%Y-%m-%d %H:%M")
      expect.eq(result, "2025-12-31 14:30")
    end)
  end)

  describe("Locale detection", function()
    before(setup)

    it("should detect locale from Accept-Language header", function()
      local result = I18n:detect_locale("en-US,en;q=0.9,fr;q=0.8")
      expect.eq(result, "en")
    end)

    it("should detect French locale", function()
      local result = I18n:detect_locale("fr-FR,fr;q=0.9,en;q=0.8")
      expect.eq(result, "fr")
    end)

    it("should fallback to default locale", function()
      local result = I18n:detect_locale("zh-CN,zh;q=0.9")
      expect.eq(result, "en") -- Chinese not loaded
    end)

    it("should handle empty header", function()
      local result = I18n:detect_locale(nil)
      expect.eq(result, "en")
    end)

    it("should handle malformed header", function()
      local result = I18n:detect_locale("invalid")
      expect.eq(result, "en")
    end)
  end)

  describe("Deep get", function()
    before(setup)

    it("should get nested value", function()
      local result = I18n:t("models.errors.presence")
      expect.eq(result, "can't be blank")
    end)

    it("should return nil for non-existent path", function()
      local result = I18n:t("nonexistent.deep.key")
      expect.eq(result, "nonexistent.deep.key")
    end)

    it("should handle empty table", function()
      I18n.translations = {}
      local result = I18n:t("any.key")
      expect.eq(result, "any.key")
    end)
  end)

  describe("Edge cases", function()
    before(setup)

    it("should handle nil key", function()
      local result = I18n:t(nil)
      expect.nil_value(result)
    end)

    it("should handle empty key", function()
      local result = I18n:t("")
      expect.eq(result, "")
    end)

    it("should handle interpolation with multiple parameters", function()
      I18n.translations.en.multi = "Hello %{name}, you have %{count} items"
      local result = I18n:t("multi", { name = "John", count = 5 })
      expect.eq(result, "Hello John, you have 5 items")
    end)

    it("should not interpolate if no parameters provided", function()
      I18n.translations.en.template = "Value: %{var}"
      local result = I18n:t("template")
      expect.eq(result, "Value: %{var}")
    end)
  end)

end)

return Test
