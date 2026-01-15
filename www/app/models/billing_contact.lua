local Model = require("model")

local BillingContact = Model.create("billing_contacts", {
  permitted_fields = {
    "owner_key", "type", "name", "email", "phone",
    "company_name", "vat_number", "registration_number",
    "billing_address", "shipping_address", "same_as_billing",
    "notes", "tags", "custom_fields",
    "total_invoiced", "total_paid", "outstanding_balance"
  },
  validations = {
    name = { presence = true }
  }
})

-- Contact types
BillingContact.TYPE_INDIVIDUAL = "individual"
BillingContact.TYPE_COMPANY = "company"

-- Get all contacts for a user
function BillingContact.for_user(owner_key, options)
  options = options or {}
  local limit = options.limit or 50
  local offset = options.offset or 0
  local search = options.search

  local query = [[
    FOR c IN billing_contacts
    FILTER c.owner_key == @owner_key OR NOT HAS(c, "owner_key") OR c.owner_key == null
  ]]

  if search and search ~= "" then
    query = query .. [[
      FILTER CONTAINS(LOWER(c.name || ''), LOWER(@search))
          OR CONTAINS(LOWER(c.email || ''), LOWER(@search))
          OR CONTAINS(LOWER(c.company_name || ''), LOWER(@search))
    ]]
  end

  query = query .. [[
    SORT c.name ASC
    LIMIT @offset, @limit
    RETURN c
  ]]

  local result = Sdb:Sdbql(query, {
    owner_key = owner_key,
    search = search,
    offset = offset,
    limit = limit
  })

  local contacts = {}
  if result and result.result then
    for _, doc in ipairs(result.result) do
      table.insert(contacts, BillingContact:new(doc))
    end
  end
  return contacts
end

-- Search contacts (for autocomplete)
function BillingContact.search(owner_key, query, limit)
  limit = limit or 10
  local result = Sdb:Sdbql([[
    FOR c IN billing_contacts
    FILTER c.owner_key == @owner_key OR NOT HAS(c, "owner_key") OR c.owner_key == null
    FILTER CONTAINS(LOWER(c.name || ''), LOWER(@query))
        OR CONTAINS(LOWER(c.email || ''), LOWER(@query))
        OR CONTAINS(LOWER(c.company_name || ''), LOWER(@query))
    LIMIT @limit
    RETURN c
  ]], { owner_key = owner_key, query = query, limit = limit })

  local contacts = {}
  if result and result.result then
    for _, doc in ipairs(result.result) do
      table.insert(contacts, BillingContact:new(doc))
    end
  end
  return contacts
end

-- Count contacts for a user
function BillingContact.count_for_user(owner_key)
  local result = Sdb:Sdbql([[
    RETURN LENGTH(FOR c IN billing_contacts FILTER c.owner_key == @owner_key OR NOT HAS(c, "owner_key") OR c.owner_key == null RETURN 1)
  ]], { owner_key = owner_key })

  if result and result.result and result.result[1] then
    return result.result[1]
  end
  return 0
end

-- Get contact's invoices
function BillingContact:get_invoices(limit)
  local key = self._key or self.data._key
  local BillingInvoice = require("models.billing_invoice")
  return BillingInvoice.for_contact(key, limit)
end

-- Get contact's quotes
function BillingContact:get_quotes(limit)
  local key = self._key or self.data._key
  local BillingQuote = require("models.billing_quote")
  return BillingQuote.for_contact(key, limit)
end

-- Update cached stats from invoices
function BillingContact:update_stats()
  local key = self._key or self.data._key
  local result = Sdb:Sdbql([[
    LET invoices = (FOR i IN billing_invoices FILTER i.contact_key == @key RETURN i)
    RETURN {
      total_invoiced: SUM(FOR i IN invoices FILTER i.status != "cancelled" RETURN i.total) || 0,
      total_paid: SUM(invoices[*].total_paid) || 0,
      outstanding: SUM(FOR i IN invoices FILTER i.status IN ["sent", "partially_paid", "overdue"] RETURN i.balance_due) || 0
    }
  ]], { key = key })

  if result and result.result and result.result[1] then
    local stats = result.result[1]
    self:update({
      total_invoiced = stats.total_invoiced,
      total_paid = stats.total_paid,
      outstanding_balance = stats.outstanding
    })
  end
end

-- Display name (company name or person name)
function BillingContact:display_name()
  local data = self.data or self
  local contact_type = data.type or "individual"
  if contact_type == "company" and data.company_name and data.company_name ~= "" then
    return data.company_name
  end
  return data.name or ""
end

-- Full address string for billing
function BillingContact:billing_address_string()
  local data = self.data or self
  local addr = data.billing_address or {}
  local parts = {}
  if addr.street and addr.street ~= "" then table.insert(parts, addr.street) end
  if addr.postal_code and addr.postal_code ~= "" then
    table.insert(parts, addr.postal_code .. " " .. (addr.city or ""))
  elseif addr.city and addr.city ~= "" then
    table.insert(parts, addr.city)
  end
  if addr.country and addr.country ~= "" then table.insert(parts, addr.country) end
  return table.concat(parts, "\n")
end

-- Get effective shipping address (billing if same_as_billing is true)
function BillingContact:effective_shipping_address()
  local data = self.data or self
  if data.same_as_billing then
    return data.billing_address or {}
  end
  return data.shipping_address or {}
end

return BillingContact
