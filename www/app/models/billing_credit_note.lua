local Model = require("model")

local BillingCreditNote = Model.create("billing_credit_notes", {
  permitted_fields = {
    "owner_key", "number", "contact_key", "invoice_key", "status",
    "date", "issued_at", "applied_at",
    "reason", "line_items",
    "subtotal", "total_tax", "total",
    "notes", "currency"
  },
  validations = {
    invoice_key = { presence = true },
    contact_key = { presence = true }
  }
})

-- Status constants
BillingCreditNote.STATUS_DRAFT = "draft"
BillingCreditNote.STATUS_ISSUED = "issued"
BillingCreditNote.STATUS_APPLIED = "applied"

-- Get credit notes for a user
function BillingCreditNote.for_user(owner_key, options)
  options = options or {}
  local limit = options.limit or 50
  local offset = options.offset or 0
  local status = options.status

  local query = [[
    FOR cn IN billing_credit_notes
    FILTER cn.owner_key == @owner_key OR NOT HAS(cn, "owner_key") OR cn.owner_key == null
  ]]

  if status and status ~= "" then
    query = query .. " FILTER cn.status == @status "
  end

  query = query .. [[
    SORT cn.date DESC
    LIMIT @offset, @limit
    RETURN cn
  ]]

  local result = Sdb:Sdbql(query, {
    owner_key = owner_key,
    status = status,
    offset = offset,
    limit = limit
  })

  local credit_notes = {}
  if result and result.result then
    for _, doc in ipairs(result.result) do
      table.insert(credit_notes, BillingCreditNote:new(doc))
    end
  end
  return credit_notes
end

-- Count credit notes by status
function BillingCreditNote.count_by_status(owner_key)
  local result = Sdb:Sdbql([[
    FOR cn IN billing_credit_notes
    COLLECT status = cn.status WITH COUNT INTO count
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

-- Get credit notes for an invoice
function BillingCreditNote.for_invoice(invoice_key)
  local result = Sdb:Sdbql([[
    FOR cn IN billing_credit_notes
    FILTER cn.invoice_key == @invoice_key
    SORT cn.date DESC
    RETURN cn
  ]], { invoice_key = invoice_key })

  local credit_notes = {}
  if result and result.result then
    for _, doc in ipairs(result.result) do
      table.insert(credit_notes, BillingCreditNote:new(doc))
    end
  end
  return credit_notes
end

-- Create credit note from invoice
function BillingCreditNote.create_from_invoice(invoice, line_items, reason)
  local BillingSettings = require("models.billing_settings")

  -- Calculate totals from line items
  local subtotal = 0
  local total_tax = 0

  for _, item in ipairs(line_items or {}) do
    local qty = tonumber(item.quantity) or 0
    local price = tonumber(item.unit_price) or 0
    local tax_rate = tonumber(item.tax_rate) or 0

    local line_subtotal = qty * price
    local tax = line_subtotal * tax_rate / 100

    item.subtotal = line_subtotal
    item.tax_amount = tax
    item.total = line_subtotal + tax

    subtotal = subtotal + line_subtotal
    total_tax = total_tax + tax
  end

  local credit_note_data = {
    owner_key = invoice.owner_key or invoice.data.owner_key,
    number = BillingSettings.next_credit_note_number(),
    contact_key = invoice.contact_key or invoice.data.contact_key,
    invoice_key = invoice._key or invoice.data._key,
    status = BillingCreditNote.STATUS_DRAFT,
    date = os.time(),
    reason = reason or "",
    line_items = line_items or {},
    subtotal = subtotal,
    total_tax = total_tax,
    total = subtotal + total_tax,
    currency = invoice.currency or invoice.data.currency or "EUR"
  }

  return BillingCreditNote:create(credit_note_data)
end

-- Calculate totals from line items
function BillingCreditNote:calculate_totals()
  local items = self.line_items or self.data.line_items or {}
  local subtotal = 0
  local total_tax = 0

  for _, item in ipairs(items) do
    local qty = tonumber(item.quantity) or 0
    local price = tonumber(item.unit_price) or 0
    local tax_rate = tonumber(item.tax_rate) or 0

    local line_subtotal = qty * price
    local tax = line_subtotal * tax_rate / 100

    item.subtotal = line_subtotal
    item.tax_amount = tax
    item.total = line_subtotal + tax

    subtotal = subtotal + line_subtotal
    total_tax = total_tax + tax
  end

  self:update({
    line_items = items,
    subtotal = subtotal,
    total_tax = total_tax,
    total = subtotal + total_tax
  })

  return self
end

-- Issue credit note
function BillingCreditNote:can_issue()
  local status = self.status or self.data.status
  return status == BillingCreditNote.STATUS_DRAFT
end

function BillingCreditNote:issue()
  if not self:can_issue() then
    return false, "Credit note can only be issued from draft status"
  end
  self:update({ status = BillingCreditNote.STATUS_ISSUED, issued_at = os.time() })
  return true
end

-- Apply credit note to invoice
function BillingCreditNote:can_apply()
  local status = self.status or self.data.status
  return status == BillingCreditNote.STATUS_ISSUED
end

function BillingCreditNote:apply()
  if not self:can_apply() then
    return false, "Credit note must be issued before applying"
  end

  local BillingInvoice = require("models.billing_invoice")
  local invoice = BillingInvoice:find(self.invoice_key or self.data.invoice_key)

  if not invoice then
    return false, "Invoice not found"
  end

  -- Add credit as negative payment to invoice
  local credit_amount = self.total or self.data.total or 0
  local number = self.number or self.data.number

  invoice:add_payment(
    -credit_amount,
    "credit_note",
    number,
    "Credit Note " .. number .. " applied"
  )

  self:update({ status = BillingCreditNote.STATUS_APPLIED, applied_at = os.time() })
  return true
end

-- Get parent invoice
function BillingCreditNote:get_invoice()
  local BillingInvoice = require("models.billing_invoice")
  local key = self.invoice_key or (self.data and self.data.invoice_key)
  if not key then return nil end
  return BillingInvoice:find(key)
end

-- Get contact
function BillingCreditNote:get_contact()
  local BillingContact = require("models.billing_contact")
  local key = self.contact_key or (self.data and self.data.contact_key)
  if not key then return nil end
  return BillingContact:find(key)
end

-- Formatted date
function BillingCreditNote:formatted_date()
  local date = self.date or self.data.date
  if not date then return "-" end
  return os.date("%d/%m/%Y", date)
end

return BillingCreditNote
