local Model = require("model")

local BillingInvoice = Model.create("billing_invoices", {
  permitted_fields = {
    "owner_key", "number", "contact_key", "quote_key", "status",
    "date", "due_date", "sent_at", "paid_at", "cancelled_at",
    "subject", "introduction", "line_items",
    "subtotal", "total_discount", "total_tax", "total",
    "total_paid", "balance_due", "payments",
    "notes", "terms", "footer", "currency",
    "cancellation_reason"
  },
  validations = {
    contact_key = { presence = true }
  }
})

-- Status constants
BillingInvoice.STATUS_DRAFT = "draft"
BillingInvoice.STATUS_SENT = "sent"
BillingInvoice.STATUS_PARTIALLY_PAID = "partially_paid"
BillingInvoice.STATUS_PAID = "paid"
BillingInvoice.STATUS_OVERDUE = "overdue"
BillingInvoice.STATUS_CANCELLED = "cancelled"
BillingInvoice.STATUS_REFUNDED = "refunded"

-- Payment methods
BillingInvoice.PAYMENT_METHODS = {
  { value = "bank_transfer", label = "Bank Transfer" },
  { value = "card", label = "Card" },
  { value = "cash", label = "Cash" },
  { value = "check", label = "Check" },
  { value = "other", label = "Other" }
}

-- Get invoices for a user
function BillingInvoice.for_user(owner_key, options)
  options = options or {}
  local limit = options.limit or 50
  local offset = options.offset or 0
  local status = options.status

  local query = [[
    FOR i IN billing_invoices
    FILTER i.owner_key == @owner_key OR NOT HAS(i, "owner_key") OR i.owner_key == null
  ]]

  if status and status ~= "" then
    query = query .. " FILTER i.status == @status "
  end

  query = query .. [[
    SORT i.date DESC
    LIMIT @offset, @limit
    RETURN i
  ]]

  local result = Sdb:Sdbql(query, {
    owner_key = owner_key,
    status = status,
    offset = offset,
    limit = limit
  })

  local invoices = {}
  if result and result.result then
    for _, doc in ipairs(result.result) do
      table.insert(invoices, BillingInvoice:new(doc))
    end
  end
  return invoices
end

-- Get invoices for a contact
function BillingInvoice.for_contact(contact_key, limit)
  limit = limit or 10
  local result = Sdb:Sdbql([[
    FOR i IN billing_invoices
    FILTER i.contact_key == @contact_key
    SORT i.date DESC
    LIMIT @limit
    RETURN i
  ]], { contact_key = contact_key, limit = limit })

  local invoices = {}
  if result and result.result then
    for _, doc in ipairs(result.result) do
      table.insert(invoices, BillingInvoice:new(doc))
    end
  end
  return invoices
end

-- Count invoices by status for a user
function BillingInvoice.count_by_status(owner_key)
  local result = Sdb:Sdbql([[
    FOR i IN billing_invoices
    COLLECT status = i.status WITH COUNT INTO count
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

-- Get dashboard stats
function BillingInvoice.stats_for_user(owner_key)
  local result = Sdb:Sdbql([[
    LET invoices = (FOR i IN billing_invoices FILTER i.owner_key == @owner_key OR NOT HAS(i, "owner_key") OR i.owner_key == null RETURN i)
    RETURN {
      total_invoiced: SUM(FOR i IN invoices FILTER i.status != "cancelled" RETURN i.total) || 0,
      total_paid: SUM(FOR i IN invoices RETURN i.total_paid) || 0,
      outstanding: SUM(FOR i IN invoices FILTER i.status IN ["sent", "partially_paid", "overdue"] RETURN i.balance_due) || 0,
      overdue_count: LENGTH(FOR i IN invoices FILTER i.status == "overdue" RETURN 1),
      draft_count: LENGTH(FOR i IN invoices FILTER i.status == "draft" RETURN 1)
    }
  ]], { owner_key = owner_key })

  if result and result.result and result.result[1] then
    return result.result[1]
  end
  return { total_invoiced = 0, total_paid = 0, outstanding = 0, overdue_count = 0, draft_count = 0 }
end

-- Create a new invoice with auto-generated number
function BillingInvoice.create_invoice(owner_key, contact_key, data)
  local BillingSettings = require("models.billing_settings")
  local settings = BillingSettings.get_default()

  local now = os.time()
  local payment_terms = settings.default_payment_terms or settings.data.default_payment_terms or 30
  local total = data.total or 0

  local invoice_data = {
    owner_key = owner_key,
    number = BillingSettings.next_invoice_number(),
    contact_key = contact_key,
    quote_key = data.quote_key,
    status = BillingInvoice.STATUS_DRAFT,
    date = data.date or now,
    due_date = data.due_date or (now + payment_terms * 86400),
    subject = data.subject or "",
    introduction = data.introduction or "",
    line_items = data.line_items or {},
    subtotal = data.subtotal or 0,
    total_discount = data.total_discount or 0,
    total_tax = data.total_tax or 0,
    total = total,
    total_paid = 0,
    balance_due = total,
    payments = {},
    notes = data.notes or "",
    terms = data.terms or "",
    footer = data.footer or "",
    currency = data.currency or settings.default_currency or settings.data.default_currency or "EUR"
  }

  return BillingInvoice:create(invoice_data)
end

-- Calculate totals from line items
function BillingInvoice:calculate_totals()
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

  local total = subtotal + total_tax
  local total_paid = self.total_paid or self.data.total_paid or 0

  self:update({
    line_items = items,
    subtotal = subtotal,
    total_discount = total_discount,
    total_tax = total_tax,
    total = total,
    balance_due = total - total_paid
  })

  return self
end

-- Add payment
function BillingInvoice:add_payment(amount, method, reference, notes, payment_date)
  local payments = self.payments or self.data.payments or {}
  local payment_id = tostring(os.time()) .. "-" .. tostring(math.random(10000, 99999))

  table.insert(payments, {
    id = payment_id,
    amount = tonumber(amount) or 0,
    method = method or "bank_transfer",
    reference = reference or "",
    date = payment_date or os.time(),
    notes = notes or ""
  })

  local total_paid = 0
  for _, p in ipairs(payments) do
    total_paid = total_paid + (tonumber(p.amount) or 0)
  end

  local total = self.total or self.data.total or 0
  local balance_due = total - total_paid
  local current_status = self.status or self.data.status
  local new_status = current_status

  if balance_due <= 0 then
    new_status = BillingInvoice.STATUS_PAID
  elseif total_paid > 0 and current_status ~= BillingInvoice.STATUS_CANCELLED then
    new_status = BillingInvoice.STATUS_PARTIALLY_PAID
  end

  local updates = {
    payments = payments,
    total_paid = total_paid,
    balance_due = balance_due,
    status = new_status
  }

  if new_status == BillingInvoice.STATUS_PAID then
    updates.paid_at = os.time()
  end

  self:update(updates)

  -- Update contact stats
  local contact = self:get_contact()
  if contact then contact:update_stats() end

  return true, payment_id
end

-- Remove payment
function BillingInvoice:remove_payment(payment_id)
  local payments = self.payments or self.data.payments or {}
  local new_payments = {}

  for _, p in ipairs(payments) do
    if p.id ~= payment_id then
      table.insert(new_payments, p)
    end
  end

  local total_paid = 0
  for _, p in ipairs(new_payments) do
    total_paid = total_paid + (tonumber(p.amount) or 0)
  end

  local total = self.total or self.data.total or 0
  local balance_due = total - total_paid
  local current_status = self.status or self.data.status
  local new_status = current_status

  if balance_due <= 0 then
    new_status = BillingInvoice.STATUS_PAID
  elseif total_paid > 0 then
    new_status = BillingInvoice.STATUS_PARTIALLY_PAID
  elseif current_status == BillingInvoice.STATUS_PAID or current_status == BillingInvoice.STATUS_PARTIALLY_PAID then
    new_status = BillingInvoice.STATUS_SENT
  end

  self:update({
    payments = new_payments,
    total_paid = total_paid,
    balance_due = balance_due,
    status = new_status,
    paid_at = (new_status == BillingInvoice.STATUS_PAID) and (self.paid_at or self.data.paid_at) or nil
  })

  -- Update contact stats
  local contact = self:get_contact()
  if contact then contact:update_stats() end

  return true
end

-- State transitions
function BillingInvoice:can_send()
  local status = self.status or self.data.status
  return status == BillingInvoice.STATUS_DRAFT
end

function BillingInvoice:send()
  if not self:can_send() then
    return false, "Invoice can only be sent from draft status"
  end
  self:update({ status = BillingInvoice.STATUS_SENT, sent_at = os.time() })

  -- Update contact stats
  local contact = self:get_contact()
  if contact then contact:update_stats() end

  return true
end

function BillingInvoice:can_cancel()
  local status = self.status or self.data.status
  return status ~= BillingInvoice.STATUS_PAID and
         status ~= BillingInvoice.STATUS_REFUNDED and
         status ~= BillingInvoice.STATUS_CANCELLED
end

function BillingInvoice:cancel(reason)
  if not self:can_cancel() then
    return false, "This invoice cannot be cancelled"
  end
  self:update({
    status = BillingInvoice.STATUS_CANCELLED,
    cancelled_at = os.time(),
    cancellation_reason = reason or ""
  })

  -- Update contact stats
  local contact = self:get_contact()
  if contact then contact:update_stats() end

  return true
end

-- Check and update overdue status
function BillingInvoice:check_overdue()
  local status = self.status or self.data.status
  local due_date = self.due_date or self.data.due_date

  if (status == BillingInvoice.STATUS_SENT or status == BillingInvoice.STATUS_PARTIALLY_PAID) and
     due_date and due_date < os.time() then
    self:update({ status = BillingInvoice.STATUS_OVERDUE })
    return true
  end
  return false
end

-- Mark all overdue invoices
function BillingInvoice.mark_overdue_invoices()
  local now = os.time()
  Sdb:Sdbql([[
    FOR i IN billing_invoices
    FILTER i.status IN ["sent", "partially_paid"]
    FILTER i.due_date < @now
    UPDATE i WITH { status: "overdue" } IN billing_invoices
  ]], { now = now })
end

-- Get contact
function BillingInvoice:get_contact()
  local BillingContact = require("models.billing_contact")
  local key = self.contact_key or (self.data and self.data.contact_key)
  if not key then return nil end
  return BillingContact:find(key)
end

-- Get quote (if created from quote)
function BillingInvoice:get_quote()
  local quote_key = self.quote_key or (self.data and self.data.quote_key)
  if not quote_key then return nil end
  local BillingQuote = require("models.billing_quote")
  return BillingQuote:find(quote_key)
end

-- Get credit notes
function BillingInvoice:get_credit_notes()
  local key = self._key or self.data._key
  local BillingCreditNote = require("models.billing_credit_note")
  return BillingCreditNote.for_invoice(key)
end

-- Formatted date
function BillingInvoice:formatted_date()
  local date = self.date or self.data.date
  if not date then return "-" end
  return os.date("%d/%m/%Y", date)
end

-- Formatted due date
function BillingInvoice:formatted_due_date()
  local date = self.due_date or self.data.due_date
  if not date then return "-" end
  return os.date("%d/%m/%Y", date)
end

-- Is overdue?
function BillingInvoice:is_overdue()
  local status = self.status or self.data.status
  local due_date = self.due_date or self.data.due_date
  if status == BillingInvoice.STATUS_OVERDUE then return true end
  if (status == BillingInvoice.STATUS_SENT or status == BillingInvoice.STATUS_PARTIALLY_PAID) and
     due_date and due_date < os.time() then
    return true
  end
  return false
end

-- Days until due (negative if overdue)
function BillingInvoice:days_until_due()
  local due_date = self.due_date or self.data.due_date
  if not due_date then return nil end
  local diff = due_date - os.time()
  return math.floor(diff / 86400)
end

-- Get monthly revenue stats for the last 12 months
function BillingInvoice.monthly_revenue_stats(owner_key)
  local one_year_ago = os.time() - (365 * 24 * 60 * 60)
  
  local result = Sdb:Sdbql([[
    FOR i IN billing_invoices
    FILTER i.owner_key == @owner_key OR NOT HAS(i, "owner_key") OR i.owner_key == null
    FILTER i.status != "cancelled" AND i.status != "draft"
    FILTER i.date >= @start_date
    RETURN { date: i.date, total: i.total, total_paid: i.total_paid }
  ]], { owner_key = owner_key, start_date = one_year_ago })
  
  return result and result.result or {}
end

return BillingInvoice
