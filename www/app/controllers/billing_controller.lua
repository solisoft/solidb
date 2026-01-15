local Controller = require("controller")
local BillingController = Controller:extend()
local AuthHelper = require("helpers.auth_helper")
local BillingSettings = require("models.billing_settings")
local BillingContact = require("models.billing_contact")
local BillingQuote = require("models.billing_quote")
local BillingInvoice = require("models.billing_invoice")
local BillingCreditNote = require("models.billing_credit_note")
local BillingPdfHelper = require("helpers.billing_pdf_helper")

-- Get current user helper
local function get_current_user()
  return AuthHelper.get_current_user()
end

-- Parse tags from comma-separated string
local function parse_tags(tags_str)
  if not tags_str or tags_str == "" then return {} end
  local tags = {}
  for tag in string.gmatch(tags_str, "[^,]+") do
    local trimmed = tag:match("^%s*(.-)%s*$")
    if trimmed and trimmed ~= "" then
      table.insert(tags, trimmed)
    end
  end
  return tags
end

-- Parse line items from form params
local function parse_line_items(params)
  local items = {}
  local i = 1
  while params["line_item_" .. i .. "_description"] do
    local item = {
      id = params["line_item_" .. i .. "_id"] or tostring(os.time()) .. "-" .. i,
      description = params["line_item_" .. i .. "_description"] or "",
      quantity = tonumber(params["line_item_" .. i .. "_quantity"]) or 1,
      unit_price = tonumber(params["line_item_" .. i .. "_unit_price"]) or 0,
      tax_rate = tonumber(params["line_item_" .. i .. "_tax_rate"]) or 0,
      discount_percent = tonumber(params["line_item_" .. i .. "_discount_percent"]) or 0
    }
    -- Calculate line totals
    local line_subtotal = item.quantity * item.unit_price
    local discount = line_subtotal * item.discount_percent / 100
    local after_discount = line_subtotal - discount
    local tax = after_discount * item.tax_rate / 100
    item.subtotal = after_discount
    item.tax_amount = tax
    item.total = after_discount + tax
    table.insert(items, item)
    i = i + 1
  end
  return items
end

-- =============================================================================
-- DASHBOARD
-- =============================================================================

function BillingController:index()
  local current_user = get_current_user()
  local settings = BillingSettings.get_default()

  -- Get stats
  local invoice_stats = BillingInvoice.stats_for_user(current_user._key)
  local quote_counts = BillingQuote.count_by_status(current_user._key)
  local contact_count = BillingContact.count_for_user(current_user._key)

  -- Recent items
  local recent_invoices = BillingInvoice.for_user(current_user._key, { limit = 5 })
  local recent_quotes = BillingQuote.for_user(current_user._key, { limit = 5 })

  self.layout = "billing"
  self:render("billing/index", {
    current_user = current_user,
    settings = settings,
    invoice_stats = invoice_stats,
    quote_counts = quote_counts,
    contact_count = contact_count,
    recent_invoices = recent_invoices,
    recent_quotes = recent_quotes,
    current_section = "dashboard"
  })
end

-- =============================================================================
-- CONTACTS
-- =============================================================================

function BillingController:contacts()
  local current_user = get_current_user()
  local page = tonumber(self.params.page) or 1
  local search = self.params.search or ""
  local limit = 25
  local offset = (page - 1) * limit

  local contacts = BillingContact.for_user(current_user._key, {
    limit = limit,
    offset = offset,
    search = search
  })
  local total = BillingContact.count_for_user(current_user._key)

  if self:is_htmx_request() and self.params.search then
    self.layout = false
    return self:render("billing/contacts/_list", {
      contacts = contacts,
      search = search
    })
  end

  self.layout = "billing"
  self:render("billing/contacts/index", {
    current_user = current_user,
    contacts = contacts,
    page = page,
    total = total,
    search = search,
    current_section = "contacts"
  })
end

function BillingController:contacts_search()
  local current_user = get_current_user()
  local query = self.params.q or ""
  local contacts = BillingContact.search(current_user._key, query, 10)

  self.layout = false
  self:render("billing/contacts/_search_results", {
    contacts = contacts,
    query = query
  })
end

function BillingController:contacts_new()
  local current_user = get_current_user()
  local settings = BillingSettings.get_default()

  self.layout = "billing"
  self:render("billing/contacts/new", {
    current_user = current_user,
    settings = settings,
    contact = {},
    current_section = "contacts"
  })
end

function BillingController:contacts_create()
  local current_user = get_current_user()

  local contact = BillingContact:create({
    owner_key = current_user._key,
    type = self.params.type or "individual",
    name = self.params.name,
    email = self.params.email or "",
    phone = self.params.phone or "",
    company_name = self.params.company_name or "",
    vat_number = self.params.vat_number or "",
    registration_number = self.params.registration_number or "",
    billing_address = {
      street = self.params.billing_street or "",
      city = self.params.billing_city or "",
      postal_code = self.params.billing_postal_code or "",
      country = self.params.billing_country or ""
    },
    shipping_address = (self.params.same_as_billing == "on") and nil or {
      street = self.params.shipping_street or "",
      city = self.params.shipping_city or "",
      postal_code = self.params.shipping_postal_code or "",
      country = self.params.shipping_country or ""
    },
    same_as_billing = self.params.same_as_billing == "on",
    notes = self.params.notes or "",
    tags = parse_tags(self.params.tags)
  })

  if contact.errors and #contact.errors > 0 then
    self:set_header("HX-Trigger", EncodeJson({ showToast = { message = contact.errors[1].message or "Validation error", type = "error" } }))
    self.layout = "billing"
    return self:render("billing/contacts/new", {
      contact = self.params,
      errors = contact.errors,
      current_section = "contacts"
    })
  end

  self:set_header("HX-Redirect", "/billing/contacts")
  self:html("")
end

function BillingController:contacts_show()
  local current_user = get_current_user()
  local contact = BillingContact:find(self.params.key)

  if not contact then
    return self:redirect("/billing/contacts")
  end

  local invoices = contact:get_invoices(10)
  local quotes = contact:get_quotes(10)

  self.layout = "billing"
  self:render("billing/contacts/show", {
    current_user = current_user,
    contact = contact,
    invoices = invoices,
    quotes = quotes,
    current_section = "contacts"
  })
end

function BillingController:contacts_edit()
  local current_user = get_current_user()
  local contact = BillingContact:find(self.params.key)

  if not contact then
    return self:redirect("/billing/contacts")
  end

  self.layout = "billing"
  self:render("billing/contacts/edit", {
    current_user = current_user,
    contact = contact,
    current_section = "contacts"
  })
end

function BillingController:contacts_update()
  local contact = BillingContact:find(self.params.key)

  if not contact then
    return self:json({ error = "Contact not found" }, 404)
  end

  contact:update({
    type = self.params.type or "individual",
    name = self.params.name,
    email = self.params.email or "",
    phone = self.params.phone or "",
    company_name = self.params.company_name or "",
    vat_number = self.params.vat_number or "",
    registration_number = self.params.registration_number or "",
    billing_address = {
      street = self.params.billing_street or "",
      city = self.params.billing_city or "",
      postal_code = self.params.billing_postal_code or "",
      country = self.params.billing_country or ""
    },
    shipping_address = (self.params.same_as_billing == "on") and nil or {
      street = self.params.shipping_street or "",
      city = self.params.shipping_city or "",
      postal_code = self.params.shipping_postal_code or "",
      country = self.params.shipping_country or ""
    },
    same_as_billing = self.params.same_as_billing == "on",
    notes = self.params.notes or "",
    tags = parse_tags(self.params.tags)
  })

  self:set_header("HX-Redirect", "/billing/contacts/" .. self.params.key)
  self:html("")
end

function BillingController:contacts_destroy()
  local contact = BillingContact:find(self.params.key)

  if contact then
    contact:destroy()
  end

  self:set_header("HX-Redirect", "/billing/contacts")
  self:html("")
end

-- =============================================================================
-- QUOTES
-- =============================================================================

function BillingController:quotes()
  local current_user = get_current_user()
  local page = tonumber(self.params.page) or 1
  local status = self.params.status or ""
  local limit = 25
  local offset = (page - 1) * limit

  local quotes = BillingQuote.for_user(current_user._key, {
    limit = limit,
    offset = offset,
    status = status
  })
  local counts = BillingQuote.count_by_status(current_user._key)

  self.layout = "billing"
  self:render("billing/quotes/index", {
    current_user = current_user,
    quotes = quotes,
    counts = counts,
    page = page,
    status = status,
    current_section = "quotes"
  })
end

function BillingController:quotes_new()
  local current_user = get_current_user()
  local settings = BillingSettings.get_default()
  local contacts = BillingContact.for_user(current_user._key, { limit = 100 })

  -- Pre-select contact if provided
  local contact_key = self.params.contact_key

  self.layout = "billing"
  self:render("billing/quotes/new", {
    current_user = current_user,
    settings = settings,
    contacts = contacts,
    contact_key = contact_key,
    quote = {},
    current_section = "quotes"
  })
end

function BillingController:quotes_create()
  local current_user = get_current_user()
  local line_items = parse_line_items(self.params)

  local quote = BillingQuote.create_quote(current_user._key, self.params.contact_key, {
    subject = self.params.subject or "",
    introduction = self.params.introduction or "",
    line_items = line_items,
    subtotal = tonumber(self.params.subtotal) or 0,
    total_discount = tonumber(self.params.total_discount) or 0,
    total_tax = tonumber(self.params.total_tax) or 0,
    total = tonumber(self.params.total) or 0,
    notes = self.params.notes or "",
    terms = self.params.terms or "",
    currency = self.params.currency or "EUR"
  })

  if quote.errors and #quote.errors > 0 then
    self:set_header("HX-Trigger", EncodeJson({ showToast = { message = quote.errors[1].message or "Validation error", type = "error" } }))
    return self:redirect("/billing/quotes/new")
  end

  self:set_header("HX-Redirect", "/billing/quotes/" .. (quote._key or quote.data._key))
  self:html("")
end

function BillingController:quotes_show()
  local current_user = get_current_user()
  local settings = BillingSettings.get_default()
  local quote = BillingQuote:find(self.params.key)

  if not quote then
    return self:redirect("/billing/quotes")
  end

  local contact = quote:get_contact()

  self.layout = "billing"
  self:render("billing/quotes/show", {
    current_user = current_user,
    settings = settings,
    quote = quote,
    contact = contact,
    current_section = "quotes"
  })
end

function BillingController:quotes_edit()
  local current_user = get_current_user()
  local settings = BillingSettings.get_default()
  local quote = BillingQuote:find(self.params.key)

  if not quote then
    return self:redirect("/billing/quotes")
  end

  local contacts = BillingContact.for_user(current_user._key, { limit = 100 })

  self.layout = "billing"
  self:render("billing/quotes/edit", {
    current_user = current_user,
    settings = settings,
    quote = quote,
    contacts = contacts,
    current_section = "quotes"
  })
end

function BillingController:quotes_update()
  local quote = BillingQuote:find(self.params.key)

  if not quote then
    return self:json({ error = "Quote not found" }, 404)
  end

  local line_items = parse_line_items(self.params)

  quote:update({
    contact_key = self.params.contact_key,
    subject = self.params.subject or "",
    introduction = self.params.introduction or "",
    line_items = line_items,
    subtotal = tonumber(self.params.subtotal) or 0,
    total_discount = tonumber(self.params.total_discount) or 0,
    total_tax = tonumber(self.params.total_tax) or 0,
    total = tonumber(self.params.total) or 0,
    notes = self.params.notes or "",
    terms = self.params.terms or "",
    currency = self.params.currency or "EUR"
  })

  self:set_header("HX-Redirect", "/billing/quotes/" .. self.params.key)
  self:html("")
end

function BillingController:quotes_destroy()
  local quote = BillingQuote:find(self.params.key)

  if quote then
    quote:destroy()
  end

  self:set_header("HX-Redirect", "/billing/quotes")
  self:html("")
end

function BillingController:quotes_send()
  local quote = BillingQuote:find(self.params.key)

  if not quote then
    return self:json({ error = "Quote not found" }, 404)
  end

  local success, err = quote:send()

  if not success then
    self:set_header("HX-Trigger", EncodeJson({ showToast = { message = err or "Cannot send quote", type = "error" } }))
    return self:html("")
  end

  self:set_header("HX-Trigger", EncodeJson({ showToast = { message = "Quote sent successfully", type = "success" } }))
  self:set_header("HX-Refresh", "true")
  self:html("")
end

function BillingController:quotes_accept()
  local quote = BillingQuote:find(self.params.key)

  if not quote then
    return self:json({ error = "Quote not found" }, 404)
  end

  local success, err = quote:accept()

  if not success then
    self:set_header("HX-Trigger", EncodeJson({ showToast = { message = err or "Cannot accept quote", type = "error" } }))
    return self:html("")
  end

  self:set_header("HX-Trigger", EncodeJson({ showToast = { message = "Quote accepted", type = "success" } }))
  self:set_header("HX-Refresh", "true")
  self:html("")
end

function BillingController:quotes_reject()
  local quote = BillingQuote:find(self.params.key)

  if not quote then
    return self:json({ error = "Quote not found" }, 404)
  end

  local success, err = quote:reject()

  if not success then
    self:set_header("HX-Trigger", EncodeJson({ showToast = { message = err or "Cannot reject quote", type = "error" } }))
    return self:html("")
  end

  self:set_header("HX-Trigger", EncodeJson({ showToast = { message = "Quote rejected", type = "success" } }))
  self:set_header("HX-Refresh", "true")
  self:html("")
end

function BillingController:quotes_convert()
  local quote = BillingQuote:find(self.params.key)

  if not quote then
    return self:json({ error = "Quote not found" }, 404)
  end

  local invoice, err = quote:convert_to_invoice()

  if not invoice then
    self:set_header("HX-Trigger", EncodeJson({ showToast = { message = err or "Cannot convert quote", type = "error" } }))
    return self:html("")
  end

  self:set_header("HX-Redirect", "/billing/invoices/" .. (invoice._key or invoice.data._key))
  self:html("")
end

function BillingController:quotes_pdf()
  local quote = BillingQuote:find(self.params.key)

  if not quote then
    return self:json({ error = "Quote not found" }, 404)
  end

  local settings = BillingSettings.get_default()
  local html = BillingPdfHelper.render_quote(quote, settings)
  local pdf, err = BillingPdfHelper.generate(html)

  if not pdf then
    return self:json({ error = err or "PDF generation failed" }, 500)
  end

  self:set_header("Content-Type", "application/pdf")
  self:set_header("Content-Disposition", "inline; filename=\"" .. (quote.number or quote.data.number) .. ".pdf\"")
  return pdf
end

-- =============================================================================
-- INVOICES
-- =============================================================================

function BillingController:invoices()
  local current_user = get_current_user()
  local page = tonumber(self.params.page) or 1
  local status = self.params.status or ""
  local limit = 25
  local offset = (page - 1) * limit

  local invoices = BillingInvoice.for_user(current_user._key, {
    limit = limit,
    offset = offset,
    status = status
  })
  local counts = BillingInvoice.count_by_status(current_user._key)

  self.layout = "billing"
  self:render("billing/invoices/index", {
    current_user = current_user,
    invoices = invoices,
    counts = counts,
    page = page,
    status = status,
    current_section = "invoices"
  })
end

function BillingController:invoices_new()
  local current_user = get_current_user()
  local settings = BillingSettings.get_default()
  local contacts = BillingContact.for_user(current_user._key, { limit = 100 })

  local contact_key = self.params.contact_key

  self.layout = "billing"
  self:render("billing/invoices/new", {
    current_user = current_user,
    settings = settings,
    contacts = contacts,
    contact_key = contact_key,
    invoice = {},
    current_section = "invoices"
  })
end

function BillingController:invoices_create()
  local current_user = get_current_user()
  local line_items = parse_line_items(self.params)

  local invoice = BillingInvoice.create_invoice(current_user._key, self.params.contact_key, {
    subject = self.params.subject or "",
    introduction = self.params.introduction or "",
    line_items = line_items,
    subtotal = tonumber(self.params.subtotal) or 0,
    total_discount = tonumber(self.params.total_discount) or 0,
    total_tax = tonumber(self.params.total_tax) or 0,
    total = tonumber(self.params.total) or 0,
    notes = self.params.notes or "",
    terms = self.params.terms or "",
    currency = self.params.currency or "EUR"
  })

  if invoice.errors and #invoice.errors > 0 then
    self:set_header("HX-Trigger", EncodeJson({ showToast = { message = invoice.errors[1].message or "Validation error", type = "error" } }))
    return self:redirect("/billing/invoices/new")
  end

  self:set_header("HX-Redirect", "/billing/invoices/" .. (invoice._key or invoice.data._key))
  self:html("")
end

function BillingController:invoices_show()
  local current_user = get_current_user()
  local settings = BillingSettings.get_default()
  local invoice = BillingInvoice:find(self.params.key)

  if not invoice then
    return self:redirect("/billing/invoices")
  end

  local contact = invoice:get_contact()
  local quote = invoice:get_quote()
  local credit_notes = invoice:get_credit_notes()

  self.layout = "billing"
  self:render("billing/invoices/show", {
    current_user = current_user,
    settings = settings,
    invoice = invoice,
    contact = contact,
    quote = quote,
    credit_notes = credit_notes,
    payment_methods = BillingInvoice.PAYMENT_METHODS,
    current_section = "invoices"
  })
end

function BillingController:invoices_edit()
  local current_user = get_current_user()
  local settings = BillingSettings.get_default()
  local invoice = BillingInvoice:find(self.params.key)

  if not invoice then
    return self:redirect("/billing/invoices")
  end

  -- Only draft invoices can be edited
  local status = invoice.status or invoice.data.status
  if status ~= "draft" then
    self:set_flash("error", "Only draft invoices can be edited")
    return self:redirect("/billing/invoices/" .. self.params.key)
  end

  local contacts = BillingContact.for_user(current_user._key, { limit = 100 })

  self.layout = "billing"
  self:render("billing/invoices/edit", {
    current_user = current_user,
    settings = settings,
    invoice = invoice,
    contacts = contacts,
    current_section = "invoices"
  })
end

function BillingController:invoices_update()
  local invoice = BillingInvoice:find(self.params.key)

  if not invoice then
    return self:json({ error = "Invoice not found" }, 404)
  end

  local status = invoice.status or invoice.data.status
  if status ~= "draft" then
    self:set_header("HX-Trigger", EncodeJson({ showToast = { message = "Only draft invoices can be edited", type = "error" } }))
    return self:html("")
  end

  local line_items = parse_line_items(self.params)

  invoice:update({
    contact_key = self.params.contact_key,
    subject = self.params.subject or "",
    introduction = self.params.introduction or "",
    line_items = line_items,
    subtotal = tonumber(self.params.subtotal) or 0,
    total_discount = tonumber(self.params.total_discount) or 0,
    total_tax = tonumber(self.params.total_tax) or 0,
    total = tonumber(self.params.total) or 0,
    balance_due = tonumber(self.params.total) or 0,
    notes = self.params.notes or "",
    terms = self.params.terms or "",
    currency = self.params.currency or "EUR"
  })

  self:set_header("HX-Redirect", "/billing/invoices/" .. self.params.key)
  self:html("")
end

function BillingController:invoices_destroy()
  local invoice = BillingInvoice:find(self.params.key)

  if invoice then
    local status = invoice.status or invoice.data.status
    if status == "draft" then
      invoice:destroy()
    end
  end

  self:set_header("HX-Redirect", "/billing/invoices")
  self:html("")
end

function BillingController:invoices_send()
  local invoice = BillingInvoice:find(self.params.key)

  if not invoice then
    return self:json({ error = "Invoice not found" }, 404)
  end

  local success, err = invoice:send()

  if not success then
    self:set_header("HX-Trigger", EncodeJson({ showToast = { message = err or "Cannot send invoice", type = "error" } }))
    return self:html("")
  end

  self:set_header("HX-Trigger", EncodeJson({ showToast = { message = "Invoice sent successfully", type = "success" } }))
  self:set_header("HX-Refresh", "true")
  self:html("")
end

function BillingController:invoices_add_payment()
  local invoice = BillingInvoice:find(self.params.key)

  if not invoice then
    return self:json({ error = "Invoice not found" }, 404)
  end

  local amount = tonumber(self.params.amount)
  if not amount or amount <= 0 then
    self:set_header("HX-Trigger", EncodeJson({ showToast = { message = "Invalid payment amount", type = "error" } }))
    return self:html("")
  end

  local success, _ = invoice:add_payment(
    amount,
    self.params.method or "bank_transfer",
    self.params.reference or "",
    self.params.notes or ""
  )

  if success then
    self:set_header("HX-Trigger", EncodeJson({ showToast = { message = "Payment recorded", type = "success" } }))
  end

  self:set_header("HX-Refresh", "true")
  self:html("")
end

function BillingController:invoices_cancel()
  local invoice = BillingInvoice:find(self.params.key)

  if not invoice then
    return self:json({ error = "Invoice not found" }, 404)
  end

  local success, err = invoice:cancel(self.params.reason or "")

  if not success then
    self:set_header("HX-Trigger", EncodeJson({ showToast = { message = err or "Cannot cancel invoice", type = "error" } }))
    return self:html("")
  end

  self:set_header("HX-Trigger", EncodeJson({ showToast = { message = "Invoice cancelled", type = "success" } }))
  self:set_header("HX-Refresh", "true")
  self:html("")
end

function BillingController:invoices_pdf()
  local invoice = BillingInvoice:find(self.params.key)

  if not invoice then
    return self:json({ error = "Invoice not found" }, 404)
  end

  local settings = BillingSettings.get_default()
  local html = BillingPdfHelper.render_invoice(invoice, settings)
  local pdf, err = BillingPdfHelper.generate(html)

  if not pdf then
    return self:json({ error = err or "PDF generation failed" }, 500)
  end

  self:set_header("Content-Type", "application/pdf")
  self:set_header("Content-Disposition", "inline; filename=\"" .. (invoice.number or invoice.data.number) .. ".pdf\"")
  return pdf
end

-- =============================================================================
-- CREDIT NOTES
-- =============================================================================

function BillingController:credit_notes()
  local current_user = get_current_user()
  local page = tonumber(self.params.page) or 1
  local status = self.params.status or ""
  local limit = 25
  local offset = (page - 1) * limit

  local credit_notes = BillingCreditNote.for_user(current_user._key, {
    limit = limit,
    offset = offset,
    status = status ~= "" and status or nil
  })

  local counts = BillingCreditNote.count_by_status(current_user._key)

  self.layout = "billing"
  self:render("billing/credit_notes/index", {
    current_user = current_user,
    credit_notes = credit_notes,
    counts = counts,
    status = status,
    page = page,
    current_section = "credit_notes"
  })
end

function BillingController:credit_notes_new()
  local current_user = get_current_user()
  local settings = BillingSettings.get_default()
  local invoice = BillingInvoice:find(self.params.invoice_key)

  if not invoice then
    return self:redirect("/billing/invoices")
  end

  local contact = invoice:get_contact()

  self.layout = "billing"
  self:render("billing/credit_notes/new", {
    current_user = current_user,
    settings = settings,
    invoice = invoice,
    contact = contact,
    current_section = "credit_notes"
  })
end

function BillingController:credit_notes_create()
  local invoice = BillingInvoice:find(self.params.invoice_key)

  if not invoice then
    return self:json({ error = "Invoice not found" }, 404)
  end

  local line_items = parse_line_items(self.params)

  local credit_note = BillingCreditNote.create_from_invoice(
    invoice,
    line_items,
    self.params.reason or ""
  )

  if credit_note.errors and #credit_note.errors > 0 then
    self:set_header("HX-Trigger", EncodeJson({ showToast = { message = credit_note.errors[1].message or "Validation error", type = "error" } }))
    return self:html("")
  end

  self:set_header("HX-Redirect", "/billing/credit-notes/" .. (credit_note._key or credit_note.data._key))
  self:html("")
end

function BillingController:credit_notes_show()
  local current_user = get_current_user()
  local settings = BillingSettings.get_default()
  local credit_note = BillingCreditNote:find(self.params.key)

  if not credit_note then
    return self:redirect("/billing/credit-notes")
  end

  local contact = credit_note:get_contact()
  local invoice = credit_note:get_invoice()

  self.layout = "billing"
  self:render("billing/credit_notes/show", {
    current_user = current_user,
    settings = settings,
    credit_note = credit_note,
    contact = contact,
    invoice = invoice,
    current_section = "credit_notes"
  })
end

function BillingController:credit_notes_issue()
  local credit_note = BillingCreditNote:find(self.params.key)

  if not credit_note then
    return self:json({ error = "Credit note not found" }, 404)
  end

  local success, err = credit_note:issue()

  if not success then
    self:set_header("HX-Trigger", EncodeJson({ showToast = { message = err or "Cannot issue credit note", type = "error" } }))
    return self:html("")
  end

  self:set_header("HX-Trigger", EncodeJson({ showToast = { message = "Credit note issued", type = "success" } }))
  self:set_header("HX-Refresh", "true")
  self:html("")
end

function BillingController:credit_notes_apply()
  local credit_note = BillingCreditNote:find(self.params.key)

  if not credit_note then
    return self:json({ error = "Credit note not found" }, 404)
  end

  local success, err = credit_note:apply()

  if not success then
    self:set_header("HX-Trigger", EncodeJson({ showToast = { message = err or "Cannot apply credit note", type = "error" } }))
    return self:html("")
  end

  self:set_header("HX-Trigger", EncodeJson({ showToast = { message = "Credit note applied to invoice", type = "success" } }))
  self:set_header("HX-Refresh", "true")
  self:html("")
end

function BillingController:credit_notes_pdf()
  local credit_note = BillingCreditNote:find(self.params.key)

  if not credit_note then
    return self:json({ error = "Credit note not found" }, 404)
  end

  local settings = BillingSettings.get_default()
  local html = BillingPdfHelper.render_credit_note(credit_note, settings)
  local pdf, err = BillingPdfHelper.generate(html)

  if not pdf then
    return self:json({ error = err or "PDF generation failed" }, 500)
  end

  self:set_header("Content-Type", "application/pdf")
  self:set_header("Content-Disposition", "inline; filename=\"" .. (credit_note.number or credit_note.data.number) .. ".pdf\"")
  return pdf
end

-- =============================================================================
-- SETTINGS
-- =============================================================================

function BillingController:settings()
  local current_user = get_current_user()
  local settings = BillingSettings.get_default()

  self.layout = "billing"
  self:render("billing/settings/index", {
    current_user = current_user,
    settings = settings,
    current_section = "settings"
  })
end

function BillingController:settings_update()
  local settings = BillingSettings.get_default()

  settings:update({
    company_name = self.params.company_name or "",
    legal_form = self.params.legal_form or "",
    vat_number = self.params.vat_number or "",
    registration_number = self.params.registration_number or "",
    company_address = self.params.company_address or "",
    company_email = self.params.company_email or "",
    company_phone = self.params.company_phone or "",
    company_website = self.params.company_website or "",
    invoice_prefix = self.params.invoice_prefix or "INV-",
    quote_prefix = self.params.quote_prefix or "QUO-",
    credit_note_prefix = self.params.credit_note_prefix or "CN-",
    default_tax_rate = tonumber(self.params.default_tax_rate) or 20,
    default_currency = self.params.default_currency or "EUR",
    payment_due_days = tonumber(self.params.payment_due_days) or 30,
    quote_validity_days = tonumber(self.params.quote_validity_days) or 30,
    payment_terms = self.params.payment_terms or "",
    bank_name = self.params.bank_name or "",
    account_holder = self.params.account_holder or "",
    iban = self.params.iban or "",
    bic = self.params.bic or ""
  })

  self:set_header("HX-Trigger", EncodeJson({ showToast = { message = "Settings saved successfully", type = "success" } }))
  self:set_header("HX-Refresh", "true")
  self:html("")
end

-- =============================================================================
-- LINE ITEMS (HTMX)
-- =============================================================================

function BillingController:line_item_new()
  local settings = BillingSettings.get_default()
  local index = tonumber(self.params.index) or 1
  local default_tax = settings.default_tax_rate or settings.data.default_tax_rate or 20

  self.layout = false
  self:render("billing/_line_item_row", {
    index = index,
    item = {
      description = "",
      quantity = 1,
      unit_price = 0,
      tax_rate = default_tax,
      discount_percent = 0,
      total = 0
    }
  })
end

return BillingController
