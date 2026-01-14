local Model = require("model")

local PageRevision = Model.create("page_revisions", {
  permitted_fields = {
    "page_id",
    "page_title",
    "change_type",    -- create, update, delete, restore
    "changed_by",     -- user key
    "changed_by_name", -- user name for display
    "changed_at",
    "content_before", -- content/blocks before change
    "content_after",  -- content/blocks after change
    "field_changed",  -- which field changed (title, content, blocks, icon, etc.)
    "summary"         -- human readable summary
  }
})

-- Create a revision entry
function PageRevision.record(page, change_type, user, opts)
  opts = opts or {}

  local user_name = "Unknown"
  if user then
    user_name = ((user.firstname or "") .. " " .. (user.lastname or "")):gsub("^%s+", ""):gsub("%s+$", "")
    if user_name == "" then user_name = user.email or "Unknown" end
  end

  local revision = PageRevision:new({
    page_id = page._key,
    page_title = page.title,
    change_type = change_type,
    changed_by = user and user._key or nil,
    changed_by_name = user_name,
    changed_at = os.time(),
    content_before = opts.before,
    content_after = opts.after,
    field_changed = opts.field,
    summary = opts.summary
  })

  revision:save()
  return revision
end

-- Get history for a page
function PageRevision.history_for(page_id, limit)
  limit = limit or 50
  local result = Sdb:Sdbql(
    "FOR r IN page_revisions FILTER r.page_id == @page_id SORT r.changed_at DESC LIMIT @limit RETURN r",
    { page_id = page_id, limit = limit }
  )

  local revisions = {}
  if result and result.result then
    for _, data in ipairs(result.result) do
      table.insert(revisions, PageRevision:new(data))
    end
  end
  return revisions
end

return PageRevision
