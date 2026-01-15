local BillingPdfHelper = {}

-- pdfx.fr API configuration
BillingPdfHelper.PDFX_API_URL = "https://pdfx.fr/api/render"

-- Format currency for display
function BillingPdfHelper.format_currency(amount, currency)
  currency = currency or "EUR"
  local symbols = { EUR = "\226\130\172", USD = "$", GBP = "\194\163" }
  local symbol = symbols[currency] or currency
  return string.format("%s %.2f", symbol, amount or 0)
end

-- Format date for display
function BillingPdfHelper.format_date(timestamp)
  if not timestamp then return "-" end
  return os.date("%d/%m/%Y", timestamp)
end

-- Generate PDF via pdfx.fr API
function BillingPdfHelper.generate(html_content, options)
  options = options or {}

  local payload = {
    html = html_content,
    format = options.format or "A4",
    landscape = options.landscape or false,
    margin = options.margin or {
      top = "20mm",
      bottom = "20mm",
      left = "15mm",
      right = "15mm"
    }
  }

  local response = Fetch(BillingPdfHelper.PDFX_API_URL, {
    method = "POST",
    headers = {
      ["Content-Type"] = "application/json"
    },
    body = EncodeJson(payload)
  })

  if response and response.status == 200 then
    return response.body
  else
    local error_msg = "PDF generation failed"
    if response then
      error_msg = error_msg .. ": " .. tostring(response.status)
      if response.body then
        error_msg = error_msg .. " - " .. tostring(response.body):sub(1, 200)
      end
    end
    return nil, error_msg
  end
end

-- Render invoice HTML template
function BillingPdfHelper.render_invoice(invoice, settings)
  local contact = invoice:get_contact()
  local line_items = invoice.line_items or invoice.data.line_items or {}
  local currency = invoice.currency or invoice.data.currency or "EUR"

  local html = BillingPdfHelper.get_base_styles() .. [[
<body>
  <div class="header">
    <div class="company-info">
      <div class="company-name">]] .. (settings.company_name or settings.data.company_name or "Your Company") .. [[</div>
      <div class="company-details">
        ]] .. (settings.company_address or settings.data.company_address or "") .. [[<br>
        ]] .. (settings.company_postal_code or settings.data.company_postal_code or "") .. [[ ]] .. (settings.company_city or settings.data.company_city or "") .. [[<br>
        ]] .. (settings.company_country or settings.data.company_country or "") .. [[
        ]] .. (settings.company_vat_number or settings.data.company_vat_number and ("<br>VAT: " .. (settings.company_vat_number or settings.data.company_vat_number)) or "") .. [[
      </div>
    </div>
    <div class="document-info">
      <div class="document-type">INVOICE</div>
      <div class="document-number">]] .. (invoice.number or invoice.data.number) .. [[</div>
    </div>
  </div>

  <div class="addresses">
    <div class="address-block">
      <div class="address-label">Bill To</div>
      <div class="address-content">
        <strong>]] .. (contact and contact:display_name() or "") .. [[</strong><br>
        ]] .. (contact and contact:billing_address_string() or "") .. [[
        ]] .. ((contact and (contact.vat_number or (contact.data and contact.data.vat_number))) and ("<br>VAT: " .. (contact.vat_number or contact.data.vat_number)) or "") .. [[
      </div>
    </div>
  </div>

  <div class="info-grid">
    <div class="info-item">
      <div class="info-label">Invoice Date</div>
      <div class="info-value">]] .. BillingPdfHelper.format_date(invoice.date or invoice.data.date) .. [[</div>
    </div>
    <div class="info-item">
      <div class="info-label">Due Date</div>
      <div class="info-value">]] .. BillingPdfHelper.format_date(invoice.due_date or invoice.data.due_date) .. [[</div>
    </div>
  </div>

  <table>
    <thead>
      <tr>
        <th style="width: 50%">Description</th>
        <th class="text-center">Qty</th>
        <th class="text-right">Unit Price</th>
        <th class="text-center">Tax</th>
        <th class="text-right">Amount</th>
      </tr>
    </thead>
    <tbody>
]]

  for _, item in ipairs(line_items) do
    html = html .. [[
      <tr>
        <td>]] .. (item.description or "") .. [[</td>
        <td class="text-center">]] .. (item.quantity or 0) .. [[</td>
        <td class="text-right">]] .. BillingPdfHelper.format_currency(item.unit_price, currency) .. [[</td>
        <td class="text-center">]] .. (item.tax_rate or 0) .. [[%</td>
        <td class="text-right">]] .. BillingPdfHelper.format_currency(item.total, currency) .. [[</td>
      </tr>
]]
  end

  html = html .. [[
    </tbody>
  </table>

  <div class="totals">
    <div class="totals-table">
      <div class="totals-row">
        <span>Subtotal</span>
        <span>]] .. BillingPdfHelper.format_currency(invoice.subtotal or invoice.data.subtotal, currency) .. [[</span>
      </div>
]]

  if (invoice.total_discount or invoice.data.total_discount or 0) > 0 then
    html = html .. [[
      <div class="totals-row">
        <span>Discount</span>
        <span>-]] .. BillingPdfHelper.format_currency(invoice.total_discount or invoice.data.total_discount, currency) .. [[</span>
      </div>
]]
  end

  html = html .. [[
      <div class="totals-row">
        <span>Tax</span>
        <span>]] .. BillingPdfHelper.format_currency(invoice.total_tax or invoice.data.total_tax, currency) .. [[</span>
      </div>
      <div class="totals-row total">
        <span>Total</span>
        <span>]] .. BillingPdfHelper.format_currency(invoice.total or invoice.data.total, currency) .. [[</span>
      </div>
    </div>
  </div>
]]

  -- Payment info
  if (settings.company_iban or settings.data.company_iban) or (settings.company_bic or settings.data.company_bic) then
    html = html .. [[
  <div class="payment-info">
    <div class="payment-title">Payment Information</div>
]]
    if settings.company_iban or settings.data.company_iban then
      html = html .. "IBAN: " .. (settings.company_iban or settings.data.company_iban) .. "<br>"
    end
    if settings.company_bic or settings.data.company_bic then
      html = html .. "BIC: " .. (settings.company_bic or settings.data.company_bic) .. "<br>"
    end
    html = html .. [[
    Please include invoice number ]] .. (invoice.number or invoice.data.number) .. [[ as reference.
  </div>
]]
  end

  -- Footer
  html = html .. [[
  <div class="footer">
]]
  if settings.pdf_terms_and_conditions or settings.data.pdf_terms_and_conditions then
    html = html .. "<p>" .. (settings.pdf_terms_and_conditions or settings.data.pdf_terms_and_conditions) .. "</p>"
  end
  if settings.pdf_footer_text or settings.data.pdf_footer_text then
    html = html .. "<p>" .. (settings.pdf_footer_text or settings.data.pdf_footer_text) .. "</p>"
  end
  html = html .. [[
  </div>
</body>
</html>
]]

  return html
end

-- Render quote HTML template
function BillingPdfHelper.render_quote(quote, settings)
  local contact = quote:get_contact()
  local line_items = quote.line_items or quote.data.line_items or {}
  local currency = quote.currency or quote.data.currency or "EUR"

  local html = BillingPdfHelper.get_base_styles() .. [[
<body>
  <div class="header">
    <div class="company-info">
      <div class="company-name">]] .. (settings.company_name or settings.data.company_name or "Your Company") .. [[</div>
      <div class="company-details">
        ]] .. (settings.company_address or settings.data.company_address or "") .. [[<br>
        ]] .. (settings.company_postal_code or settings.data.company_postal_code or "") .. [[ ]] .. (settings.company_city or settings.data.company_city or "") .. [[<br>
        ]] .. (settings.company_country or settings.data.company_country or "") .. [[
      </div>
    </div>
    <div class="document-info">
      <div class="document-type" style="color: #10b981;">QUOTE</div>
      <div class="document-number">]] .. (quote.number or quote.data.number) .. [[</div>
    </div>
  </div>

  <div class="addresses">
    <div class="address-block">
      <div class="address-label">To</div>
      <div class="address-content">
        <strong>]] .. (contact and contact:display_name() or "") .. [[</strong><br>
        ]] .. (contact and contact:billing_address_string() or "") .. [[
      </div>
    </div>
  </div>

  <div class="info-grid">
    <div class="info-item">
      <div class="info-label">Quote Date</div>
      <div class="info-value">]] .. BillingPdfHelper.format_date(quote.date or quote.data.date) .. [[</div>
    </div>
    <div class="info-item">
      <div class="info-label">Valid Until</div>
      <div class="info-value">]] .. BillingPdfHelper.format_date(quote.valid_until or quote.data.valid_until) .. [[</div>
    </div>
  </div>
]]

  if quote.subject or quote.data.subject then
    html = html .. [[
  <div class="subject">
    <strong>Subject:</strong> ]] .. (quote.subject or quote.data.subject) .. [[
  </div>
]]
  end

  if quote.introduction or quote.data.introduction then
    html = html .. [[
  <div class="introduction">
    ]] .. (quote.introduction or quote.data.introduction) .. [[
  </div>
]]
  end

  html = html .. [[
  <table>
    <thead>
      <tr>
        <th style="width: 50%">Description</th>
        <th class="text-center">Qty</th>
        <th class="text-right">Unit Price</th>
        <th class="text-center">Tax</th>
        <th class="text-right">Amount</th>
      </tr>
    </thead>
    <tbody>
]]

  for _, item in ipairs(line_items) do
    html = html .. [[
      <tr>
        <td>]] .. (item.description or "") .. [[</td>
        <td class="text-center">]] .. (item.quantity or 0) .. [[</td>
        <td class="text-right">]] .. BillingPdfHelper.format_currency(item.unit_price, currency) .. [[</td>
        <td class="text-center">]] .. (item.tax_rate or 0) .. [[%</td>
        <td class="text-right">]] .. BillingPdfHelper.format_currency(item.total, currency) .. [[</td>
      </tr>
]]
  end

  html = html .. [[
    </tbody>
  </table>

  <div class="totals">
    <div class="totals-table">
      <div class="totals-row">
        <span>Subtotal</span>
        <span>]] .. BillingPdfHelper.format_currency(quote.subtotal or quote.data.subtotal, currency) .. [[</span>
      </div>
      <div class="totals-row">
        <span>Tax</span>
        <span>]] .. BillingPdfHelper.format_currency(quote.total_tax or quote.data.total_tax, currency) .. [[</span>
      </div>
      <div class="totals-row total">
        <span>Total</span>
        <span>]] .. BillingPdfHelper.format_currency(quote.total or quote.data.total, currency) .. [[</span>
      </div>
    </div>
  </div>

  <div class="footer">
    <p>This quote is valid until ]] .. BillingPdfHelper.format_date(quote.valid_until or quote.data.valid_until) .. [[.</p>
]]

  if settings.pdf_terms_and_conditions or settings.data.pdf_terms_and_conditions then
    html = html .. "<p>" .. (settings.pdf_terms_and_conditions or settings.data.pdf_terms_and_conditions) .. "</p>"
  end

  html = html .. [[
  </div>
</body>
</html>
]]

  return html
end

-- Render credit note HTML template
function BillingPdfHelper.render_credit_note(credit_note, settings)
  local contact = credit_note:get_contact()
  local invoice = credit_note:get_invoice()
  local line_items = credit_note.line_items or credit_note.data.line_items or {}
  local currency = credit_note.currency or credit_note.data.currency or "EUR"

  local html = BillingPdfHelper.get_base_styles() .. [[
<body>
  <div class="header">
    <div class="company-info">
      <div class="company-name">]] .. (settings.company_name or settings.data.company_name or "Your Company") .. [[</div>
      <div class="company-details">
        ]] .. (settings.company_address or settings.data.company_address or "") .. [[<br>
        ]] .. (settings.company_postal_code or settings.data.company_postal_code or "") .. [[ ]] .. (settings.company_city or settings.data.company_city or "") .. [[<br>
        ]] .. (settings.company_country or settings.data.company_country or "") .. [[
      </div>
    </div>
    <div class="document-info">
      <div class="document-type" style="color: #ef4444;">CREDIT NOTE</div>
      <div class="document-number">]] .. (credit_note.number or credit_note.data.number) .. [[</div>
    </div>
  </div>

  <div class="addresses">
    <div class="address-block">
      <div class="address-label">To</div>
      <div class="address-content">
        <strong>]] .. (contact and contact:display_name() or "") .. [[</strong><br>
        ]] .. (contact and contact:billing_address_string() or "") .. [[
      </div>
    </div>
  </div>

  <div class="info-grid">
    <div class="info-item">
      <div class="info-label">Date</div>
      <div class="info-value">]] .. BillingPdfHelper.format_date(credit_note.date or credit_note.data.date) .. [[</div>
    </div>
    <div class="info-item">
      <div class="info-label">Related Invoice</div>
      <div class="info-value">]] .. (invoice and (invoice.number or invoice.data.number) or "-") .. [[</div>
    </div>
  </div>
]]

  if credit_note.reason or credit_note.data.reason then
    html = html .. [[
  <div class="reason">
    <strong>Reason:</strong> ]] .. (credit_note.reason or credit_note.data.reason) .. [[
  </div>
]]
  end

  html = html .. [[
  <table>
    <thead>
      <tr>
        <th style="width: 50%">Description</th>
        <th class="text-center">Qty</th>
        <th class="text-right">Unit Price</th>
        <th class="text-center">Tax</th>
        <th class="text-right">Amount</th>
      </tr>
    </thead>
    <tbody>
]]

  for _, item in ipairs(line_items) do
    html = html .. [[
      <tr>
        <td>]] .. (item.description or "") .. [[</td>
        <td class="text-center">]] .. (item.quantity or 0) .. [[</td>
        <td class="text-right">]] .. BillingPdfHelper.format_currency(item.unit_price, currency) .. [[</td>
        <td class="text-center">]] .. (item.tax_rate or 0) .. [[%</td>
        <td class="text-right">]] .. BillingPdfHelper.format_currency(item.total, currency) .. [[</td>
      </tr>
]]
  end

  html = html .. [[
    </tbody>
  </table>

  <div class="totals">
    <div class="totals-table">
      <div class="totals-row">
        <span>Subtotal</span>
        <span>]] .. BillingPdfHelper.format_currency(credit_note.subtotal or credit_note.data.subtotal, currency) .. [[</span>
      </div>
      <div class="totals-row">
        <span>Tax</span>
        <span>]] .. BillingPdfHelper.format_currency(credit_note.total_tax or credit_note.data.total_tax, currency) .. [[</span>
      </div>
      <div class="totals-row total" style="color: #ef4444;">
        <span>Credit Total</span>
        <span>-]] .. BillingPdfHelper.format_currency(credit_note.total or credit_note.data.total, currency) .. [[</span>
      </div>
    </div>
  </div>

  <div class="footer">
    <p>This credit note will be applied to invoice ]] .. (invoice and (invoice.number or invoice.data.number) or "-") .. [[.</p>
  </div>
</body>
</html>
]]

  return html
end

-- Base CSS styles for all PDF documents
function BillingPdfHelper.get_base_styles()
  return [[
<!DOCTYPE html>
<html>
<head>
  <meta charset="UTF-8">
  <style>
    * { box-sizing: border-box; margin: 0; padding: 0; }
    body { font-family: 'Helvetica Neue', Arial, sans-serif; font-size: 12px; color: #333; line-height: 1.5; padding: 20px; }

    .header { display: flex; justify-content: space-between; margin-bottom: 40px; }
    .company-info { }
    .company-name { font-size: 24px; font-weight: bold; color: #1a1a2e; }
    .company-details { color: #666; margin-top: 8px; font-size: 11px; }

    .document-info { text-align: right; }
    .document-type { font-size: 28px; font-weight: bold; color: #3b82f6; }
    .document-number { font-size: 14px; color: #666; margin-top: 4px; }

    .addresses { display: flex; justify-content: space-between; margin-bottom: 30px; }
    .address-block { width: 45%; }
    .address-label { font-size: 10px; text-transform: uppercase; color: #999; margin-bottom: 8px; letter-spacing: 0.5px; }
    .address-content { background: #f8f9fa; padding: 16px; border-radius: 4px; font-size: 11px; }

    .info-grid { display: flex; gap: 40px; margin-bottom: 30px; }
    .info-item { }
    .info-label { font-size: 10px; text-transform: uppercase; color: #999; letter-spacing: 0.5px; }
    .info-value { font-size: 13px; font-weight: 500; margin-top: 4px; }

    .subject { margin-bottom: 15px; font-size: 13px; }
    .introduction { margin-bottom: 20px; color: #555; white-space: pre-line; }
    .reason { margin-bottom: 20px; font-size: 12px; background: #fef3c7; padding: 12px; border-radius: 4px; }

    table { width: 100%; border-collapse: collapse; margin-bottom: 30px; }
    th { background: #1a1a2e; color: white; padding: 10px 12px; text-align: left; font-size: 10px; text-transform: uppercase; letter-spacing: 0.5px; }
    td { padding: 10px 12px; border-bottom: 1px solid #eee; font-size: 11px; }
    .text-right { text-align: right; }
    .text-center { text-align: center; }

    .totals { display: flex; justify-content: flex-end; margin-bottom: 30px; }
    .totals-table { width: 280px; }
    .totals-row { display: flex; justify-content: space-between; padding: 8px 0; border-bottom: 1px solid #eee; font-size: 12px; }
    .totals-row.total { font-size: 16px; font-weight: bold; border-bottom: 2px solid #1a1a2e; padding: 12px 0; }

    .payment-info { background: #f0f9ff; padding: 16px; border-radius: 4px; margin-bottom: 20px; font-size: 11px; }
    .payment-title { font-weight: bold; color: #3b82f6; margin-bottom: 8px; font-size: 12px; }

    .footer { margin-top: 40px; padding-top: 20px; border-top: 1px solid #eee; color: #666; font-size: 10px; }
    .footer p { margin-bottom: 8px; }
  </style>
</head>
]]
end

return BillingPdfHelper
