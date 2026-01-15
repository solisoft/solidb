local Model = require("model")

local BillingSettings = Model.create("billing_settings", {
  permitted_fields = {
    "company_name", "legal_form", "company_address", "company_email", "company_phone", "company_website",
    "vat_number", "registration_number",
    "bank_name", "account_holder", "iban", "bic",
    "invoice_prefix", "invoice_next_number", "invoice_year", "invoice_year_reset",
    "quote_prefix", "quote_next_number", "quote_year", "quote_year_reset",
    "credit_note_prefix", "credit_note_next_number", "credit_note_year", "credit_note_year_reset",
    "default_tax_rate", "default_currency", "payment_due_days", "quote_validity_days",
    "payment_terms"
  }
})

-- Get or create default settings
function BillingSettings.get_default()
  local result = Sdb:Sdbql([[
    FOR s IN billing_settings
    FILTER s._key == "default"
    LIMIT 1
    RETURN s
  ]])

  if result and result.result and result.result[1] then
    return BillingSettings:new(result.result[1])
  end

  -- Create default settings
  local settings = BillingSettings:create({
    _key = "default",
    invoice_prefix = "INV-",
    invoice_next_number = 1,
    quote_prefix = "QUO-",
    quote_next_number = 1,
    credit_note_prefix = "CN-",
    credit_note_next_number = 1,
    default_tax_rate = 20.0,
    payment_due_days = 30,
    default_currency = "EUR",
    quote_validity_days = 30
  })

  return settings
end

-- Generate next invoice number with atomic increment
function BillingSettings.next_invoice_number()
  local current_year = os.date("%Y")
  local result = Sdb:Sdbql([[
    UPSERT { _key: "default" }
    INSERT {
      _key: "default",
      invoice_prefix: "INV-",
      invoice_next_number: 2,
      invoice_year: @year
    }
    UPDATE {
      invoice_next_number: (OLD.invoice_year != @year && OLD.invoice_year_reset) ? 2 : OLD.invoice_next_number + 1,
      invoice_year: @year
    }
    IN billing_settings
    RETURN {
      prefix: OLD.invoice_prefix || "INV-",
      number: (OLD.invoice_year != @year && OLD.invoice_year_reset) ? 1 : (OLD.invoice_next_number || 1),
      year: @year
    }
  ]], { year = current_year })

  if result and result.result and result.result[1] then
    local r = result.result[1]
    return r.prefix .. r.year .. "-" .. string.format("%04d", r.number)
  end
  return "INV-" .. current_year .. "-" .. os.time()
end

-- Generate next quote number with atomic increment
function BillingSettings.next_quote_number()
  local current_year = os.date("%Y")
  local result = Sdb:Sdbql([[
    UPSERT { _key: "default" }
    INSERT {
      _key: "default",
      quote_prefix: "QUO-",
      quote_next_number: 2,
      quote_year: @year
    }
    UPDATE {
      quote_next_number: (OLD.quote_year != @year && OLD.quote_year_reset) ? 2 : OLD.quote_next_number + 1,
      quote_year: @year
    }
    IN billing_settings
    RETURN {
      prefix: OLD.quote_prefix || "QUO-",
      number: (OLD.quote_year != @year && OLD.quote_year_reset) ? 1 : (OLD.quote_next_number || 1),
      year: @year
    }
  ]], { year = current_year })

  if result and result.result and result.result[1] then
    local r = result.result[1]
    return r.prefix .. r.year .. "-" .. string.format("%04d", r.number)
  end
  return "QUO-" .. current_year .. "-" .. os.time()
end

-- Generate next credit note number with atomic increment
function BillingSettings.next_credit_note_number()
  local current_year = os.date("%Y")
  local result = Sdb:Sdbql([[
    UPSERT { _key: "default" }
    INSERT {
      _key: "default",
      credit_note_prefix: "CN-",
      credit_note_next_number: 2,
      credit_note_year: @year
    }
    UPDATE {
      credit_note_next_number: (OLD.credit_note_year != @year && OLD.credit_note_year_reset) ? 2 : OLD.credit_note_next_number + 1,
      credit_note_year: @year
    }
    IN billing_settings
    RETURN {
      prefix: OLD.credit_note_prefix || "CN-",
      number: (OLD.credit_note_year != @year && OLD.credit_note_year_reset) ? 1 : (OLD.credit_note_next_number || 1),
      year: @year
    }
  ]], { year = current_year })

  if result and result.result and result.result[1] then
    local r = result.result[1]
    return r.prefix .. r.year .. "-" .. string.format("%04d", r.number)
  end
  return "CN-" .. current_year .. "-" .. os.time()
end

return BillingSettings
