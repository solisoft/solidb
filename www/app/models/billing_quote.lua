local Model = require("model")

local BillingQuote = Model.create("billing_quotes", {
  permitted_fields = {
    "owner_key", "number", "contact_key", "status",
    "date", "valid_until", "sent_at", "accepted_at", "rejected_at",
    "subject", "introduction", "line_items",
    "subtotal", "total_discount", "total_tax", "total",
    "notes", "terms", "footer", "currency",
    "converted_to_invoice_key"
  },
  validations = {
    contact_key = { presence = true }
  }
})

-- Status constants
BillingQuote.STATUS_DRAFT = "draft"
BillingQuote.STATUS_SENT = "sent"
BillingQuote.STATUS_ACCEPTED = "accepted"
BillingQuote.STATUS_REJECTED = "rejected"
BillingQuote.STATUS_EXPIRED = "expired"
BillingQuote.STATUS_CONVERTED = "converted"

-- Get quotes for a user
function BillingQuote.for_user(owner_key, options)
  options = options or {}
  local limit = options.limit or 50
  local offset = options.offset or 0
  local status = options.status

  local query = [[
    FOR q IN billing_quotes
    FILTER q.owner_key == @owner_key OR NOT HAS(q, "owner_key") OR q.owner_key == null
  ]]

  if status and status ~= "" then
    query = query .. " FILTER q.status == @status "
  end

  query = query .. [[
    SORT q.date DESC
    LIMIT @offset, @limit
    RETURN q
  ]]

  local result = Sdb:Sdbql(query, {
    owner_key = owner_key,
    status = status,
    offset = offset,
    limit = limit
  })

  local quotes = {}
  if result and result.result then
    for _, doc in ipairs(result.result) do
      table.insert(quotes, BillingQuote:new(doc))
    end
  end
  return quotes
end

-- Get quotes for a contact
function BillingQuote.for_contact(contact_key, limit)
  limit = limit or 10
  local result = Sdb:Sdbql([[
    FOR q IN billing_quotes
    FILTER q.contact_key == @contact_key
    SORT q.date DESC
    LIMIT @limit
    RETURN q
  ]], { contact_key = contact_key, limit = limit })

  local quotes = {}
  if result and result.result then
    for _, doc in ipairs(result.result) do
      table.insert(quotes, BillingQuote:new(doc))
    end
  end
  return quotes
end

-- Count quotes by status for a user
function BillingQuote.count_by_status(owner_key)
  local result = Sdb:Sdbql([[
    FOR q IN billing_quotes
    COLLECT status = q.status WITH COUNT INTO count
    RETURN { status: status, count: count }
  ]])

  local counts = { total = 0 }
  if result and result.result then
    for _, r in ipairs(result.result) do
      if r.status then
        counts[r.status] = r.count
        counts.total = counts.total + r.count
      end
    end
  end
  return counts
end

-- Create a new quote with auto-generated number
function BillingQuote.create_quote(owner_key, contact_key, data)
  local BillingSettings = require("models.billing_settings")
  local settings = BillingSettings.get_default()

  local now = os.time()
  local validity_days = settings.default_quote_validity or settings.data.default_quote_validity or 30

  local quote_data = {
    owner_key = owner_key,
    number = BillingSettings.next_quote_number(),
    contact_key = contact_key,
    status = BillingQuote.STATUS_DRAFT,
    date = data.date or now,
    valid_until = data.valid_until or (now + validity_days * 86400),
    subject = data.subject or "",
    introduction = data.introduction or "",
    line_items = data.line_items or {},
    subtotal = data.subtotal or 0,
    total_discount = data.total_discount or 0,
    total_tax = data.total_tax or 0,
    total = data.total or 0,
    notes = data.notes or "",
    terms = data.terms or "",
    footer = data.footer or "",
    currency = data.currency or settings.default_currency or settings.data.default_currency or "EUR"
  }

  return BillingQuote:create(quote_data)
end

-- Calculate totals from line items
function BillingQuote:calculate_totals()
  local items = self.line_items or self.data.line_items or {}
  local subtotal = 0
  local total_tax = 0
  local total_discount = 0

  for _, item in ipairs(items) do
    local qty = tonumber(item.quantity) or 0
    local price = tonumber(item.unit_price) or 0
    local tax_rate = tonumber(item.tax_rate) or 0
    local discount_percent = tonumber(item.discount_percent) or 0

    local line_subtotal = qty * price
    local discount = line_subtotal * discount_percent / 100
    local after_discount = line_subtotal - discount
    local tax = after_discount * tax_rate / 100

    item.subtotal = after_discount
    item.tax_amount = tax
    item.total = after_discount + tax

    subtotal = subtotal + after_discount
    total_tax = total_tax + tax
    total_discount = total_discount + discount
  end

  self:update({
    line_items = items,
    subtotal = subtotal,
    total_discount = total_discount,
    total_tax = total_tax,
    total = subtotal + total_tax
  })

  return self
end

-- State transitions
function BillingQuote:can_send()
  local status = self.status or self.data.status
  return status == BillingQuote.STATUS_DRAFT
end

function BillingQuote:send()
  if not self:can_send() then
    return false, "Quote can only be sent from draft status"
  end
  self:update({ status = BillingQuote.STATUS_SENT, sent_at = os.time() })
  return true
end

function BillingQuote:can_accept()
  local status = self.status or self.data.status
  return status == BillingQuote.STATUS_SENT
end

function BillingQuote:accept()
  if not self:can_accept() then
    return false, "Quote can only be accepted from sent status"
  end
  self:update({ status = BillingQuote.STATUS_ACCEPTED, accepted_at = os.time() })
  return true
end

function BillingQuote:can_reject()
  local status = self.status or self.data.status
  return status == BillingQuote.STATUS_SENT
end

function BillingQuote:reject()
  if not self:can_reject() then
    return false, "Quote can only be rejected from sent status"
  end
  self:update({ status = BillingQuote.STATUS_REJECTED, rejected_at = os.time() })
  return true
end

-- Check if quote is expired
function BillingQuote:check_expired()
  local status = self.status or self.data.status
  local valid_until = self.valid_until or self.data.valid_until

  if status == BillingQuote.STATUS_SENT and valid_until and valid_until < os.time() then
    self:update({ status = BillingQuote.STATUS_EXPIRED })
    return true
  end
  return false
end

-- Convert quote to invoice
function BillingQuote:can_convert()
  local status = self.status or self.data.status
  return status == BillingQuote.STATUS_ACCEPTED
end

function BillingQuote:convert_to_invoice()
  if not self:can_convert() then
    return nil, "Quote must be accepted before converting to invoice"
  end

  local BillingInvoice = require("models.billing_invoice")

  local invoice = BillingInvoice.create_invoice(
    self.owner_key or self.data.owner_key,
    self.contact_key or self.data.contact_key,
    {
      quote_key = self._key or self.data._key,
      subject = self.subject or self.data.subject,
      introduction = self.introduction or self.data.introduction,
      line_items = self.line_items or self.data.line_items,
      subtotal = self.subtotal or self.data.subtotal,
      total_discount = self.total_discount or self.data.total_discount,
      total_tax = self.total_tax or self.data.total_tax,
      total = self.total or self.data.total,
      notes = self.notes or self.data.notes,
      terms = self.terms or self.data.terms,
      footer = self.footer or self.data.footer,
      currency = self.currency or self.data.currency
    }
  )

  if invoice and (invoice._key or (invoice.data and invoice.data._key)) then
    -- Set invoice to sent status since it comes from an accepted quote
    invoice:update({
      status = BillingInvoice.STATUS_SENT,
      sent_at = os.time()
    })

    self:update({
      status = BillingQuote.STATUS_CONVERTED,
      converted_to_invoice_key = invoice._key or invoice.data._key
    })
  end

  return invoice
end

-- Get contact
function BillingQuote:get_contact()
  local BillingContact = require("models.billing_contact")
  local key = self.contact_key or (self.data and self.data.contact_key)
  if not key then return nil end
  return BillingContact:find(key)
end

-- Formatted date
function BillingQuote:formatted_date()
  local date = self.date or self.data.date
  if not date then return "-" end
  return os.date("%d/%m/%Y", date)
end

-- Formatted valid until
function BillingQuote:formatted_valid_until()
  local date = self.valid_until or self.data.valid_until
  if not date then return "-" end
  return os.date("%d/%m/%Y", date)
end

-- Is expired?
function BillingQuote:is_expired()
  local valid_until = self.valid_until or self.data.valid_until
  if not valid_until then return false end
  return valid_until < os.time()
end

return BillingQuote
