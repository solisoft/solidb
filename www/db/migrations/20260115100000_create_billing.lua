local M = {}

function M.up(db, helpers)
  -- Settings collection (single document for app config)
  helpers.create_collection("billing_settings")

  -- Contacts collection
  helpers.create_collection("billing_contacts")
  helpers.add_index("billing_contacts", { "owner_key" }, { name = "idx_contacts_owner" })
  helpers.add_index("billing_contacts", { "email" }, { name = "idx_contacts_email" })

  -- Quotes collection
  helpers.create_collection("billing_quotes")
  helpers.add_index("billing_quotes", { "owner_key" }, { name = "idx_quotes_owner" })
  helpers.add_index("billing_quotes", { "contact_key" }, { name = "idx_quotes_contact" })
  helpers.add_index("billing_quotes", { "status" }, { name = "idx_quotes_status" })
  helpers.add_index("billing_quotes", { "number" }, { name = "idx_quotes_number" })

  -- Invoices collection
  helpers.create_collection("billing_invoices")
  helpers.add_index("billing_invoices", { "owner_key" }, { name = "idx_invoices_owner" })
  helpers.add_index("billing_invoices", { "contact_key" }, { name = "idx_invoices_contact" })
  helpers.add_index("billing_invoices", { "status" }, { name = "idx_invoices_status" })
  helpers.add_index("billing_invoices", { "number" }, { name = "idx_invoices_number" })
  helpers.add_index("billing_invoices", { "quote_key" }, { name = "idx_invoices_quote" })

  -- Credit Notes collection
  helpers.create_collection("billing_credit_notes")
  helpers.add_index("billing_credit_notes", { "owner_key" }, { name = "idx_cn_owner" })
  helpers.add_index("billing_credit_notes", { "contact_key" }, { name = "idx_cn_contact" })
  helpers.add_index("billing_credit_notes", { "invoice_key" }, { name = "idx_cn_invoice" })
  helpers.add_index("billing_credit_notes", { "number" }, { name = "idx_cn_number" })
end

function M.down(db, helpers)
  helpers.drop_collection("billing_credit_notes")
  helpers.drop_collection("billing_invoices")
  helpers.drop_collection("billing_quotes")
  helpers.drop_collection("billing_contacts")
  helpers.drop_collection("billing_settings")
end

return M
