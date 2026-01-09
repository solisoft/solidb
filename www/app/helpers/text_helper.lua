local M = {}

function M.escape_html(s)
  if not s then return "" end
  return (s:gsub("[&<>'\"]", {
    ["&"] = "&amp;",
    ["<"] = "&lt;",
    [">"] = "&gt;",
    ["'"] = "&#39;",
    ["\""] = "&quot;"
  }))
end

function M.is_only_emojis(str)
  if not str or str == "" then return false end
  -- Remove whitespace
  local s = str:gsub("%s+", "")
  if s == "" then return false end
  -- Max length for emoji-only messages
  if #s > 30 then return false end
  -- Must not contain alphanumeric chars
  if s:find("[0-9a-zA-Z]") then return false end
  -- Must not contain common ASCII punctuation/symbols (not emojis)
  if s:find("[%?!%.,%-%+%*/:;'\"@#$%%^&%(%)%[%]{}|\\<>=_~`]") then return false end
  -- Must have at least one non-ASCII char (actual emoji)
  -- Check for bytes > 127 which indicates multi-byte UTF-8 (emojis)
  for i = 1, #s do
    if s:byte(i) > 127 then return true end
  end
  return false
end

function M.format_message(text)
  if not text then return "" end
  local escaped = M.escape_html(text)
  
  -- Mask code blocks to protect them from other formatting
  local code_blocks = {}
  local code_block_placeholder = "_{{CODE_BLOCK_%d}}_"
  local masked_text = escaped:gsub("```(%w*)%s*[\n\r]?([%s%S]-)%s*```", function(lang, code)
    local index = #code_blocks + 1
    local lang_class = ""
    if lang and lang ~= "" then
      lang_class = " language-" .. lang:lower()
    end
    -- Using theme classes matching talks app (bg-bg-dark, border-border/30)
    local pre_class = ' class="bg-bg-dark/50 border border-border/30 rounded-lg p-3 overflow-x-auto my-2 text-sm font-mono"'
    local code_class = ' class="font-mono text-sm' .. lang_class .. '"'
    -- Preserve newlines inside code block by encoding them temporarily if needed, 
    -- but since we restore the whole block at the end, standard newlines are fine 
    -- provided the intermediate steps don't strip them.
    -- However, the blockquote processor splits by \n.
    -- So we should probably keep the placeholder on a single line or ensure it handles it.
    -- Best to replacing the whole block with a placeholder string.
    table.insert(code_blocks, '<pre' .. pre_class .. '><code' .. code_class .. '>' .. code .. '</code></pre>')
    return string.format(code_block_placeholder, index)
  end)
  
  -- Mask inline code as well
  local inline_code_blocks = {}
  local inline_code_placeholder = "_{{INLINE_CODE_%d}}_"
  masked_text = masked_text:gsub("`([^`]+)`", function(code)
      local index = #inline_code_blocks + 1
      table.insert(inline_code_blocks, "<code class='bg-bg-dark px-1.5 py-0.5 rounded text-sm font-mono text-primary-light'>" .. code .. "</code>")
      return string.format(inline_code_placeholder, index)
  end)
  
  local formatted = masked_text

  -- Gallery Grid: :::gallery ... :::
  -- Wraps multiple images in a responsive grid layout
  formatted = formatted:gsub(":::gallery%s*([%s%S]-)%s*:::", function(content)
      -- Format images inside the gallery with w-full for grid cells
      local gallery_images = content:gsub("!%[([^%]]*)%]%(([^%)]+)%)", '<img src="%2" alt="%1" onclick="openLightbox(this.src)" class="w-full h-32 object-cover rounded-lg border border-border/30 cursor-pointer hover:opacity-90 transition-opacity">')
      
      -- Remove any leftover newlines/spaces between images to keep grid clean (optional, HTML handles it, but safer)
      -- Actually, we just wrap it
      return '<div class="grid grid-cols-2 md:grid-cols-3 lg:grid-cols-4 gap-2 my-2">' .. gallery_images .. '</div>'
  end)

  -- Files Grid: :::files ... :::
  -- Wraps multiple file attachments in a grid layout
  formatted = formatted:gsub(":::files%s*([%s%S]-)%s*:::", function(content)
      return '<div class="grid grid-cols-1 md:grid-cols-2 gap-2 my-2">' .. content .. '</div>'
  end)

  -- Images: ![alt](url)
  formatted = formatted:gsub("!%[([^%]]*)%]%(([^%)]+)%)", '<img src="%2" alt="%1" onclick="openLightbox(this.src)" class="max-w-md max-h-96 rounded-lg border border-border/30 my-2 cursor-pointer hover:opacity-90 transition-opacity">')

  -- File Attachments: [name](/talks/file/key)
  -- Render as a nice card with icon
  formatted = formatted:gsub("%[([^%]]+)%]%((/talks/file/[^%)]+)%)", function(name, url)
      return '<a href="' .. url .. '" target="_blank" rel="noopener" class="inline-flex items-center gap-3 px-4 py-3 my-2 bg-gradient-to-r from-bg-dark/80 to-bg-dark/40 border border-border/40 rounded-xl hover:border-primary/50 hover:shadow-lg hover:shadow-primary/10 transition-all duration-200 group max-w-sm">' ..
             '<div class="flex-shrink-0 w-10 h-10 bg-gradient-to-br from-primary/20 to-accent/20 rounded-lg flex items-center justify-center">' ..
               '<svg xmlns="http://www.w3.org/2000/svg" class="h-5 w-5 text-primary" fill="none" viewBox="0 0 24 24" stroke="currentColor">' ..
                 '<path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z" />' ..
               '</svg>' ..
             '</div>' ..
             '<div class="flex-1 min-w-0">' ..
               '<div class="font-medium text-text text-sm truncate group-hover:text-primary transition-colors">' .. name .. '</div>' ..
               '<div class="text-xs text-text-muted">Click to download</div>' ..
             '</div>' ..
             '<div class="flex-shrink-0 text-text-muted group-hover:text-primary transition-colors">' ..
               '<svg xmlns="http://www.w3.org/2000/svg" class="h-5 w-5" fill="none" viewBox="0 0 24 24" stroke="currentColor">' ..
                 '<path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-4l-4 4m0 0l-4-4m4 4V4" />' ..
               '</svg>' ..
             '</div>' ..
             '</a>'
  end)

  -- Links: [text](url)
  formatted = formatted:gsub("%[([^%]]+)%]%(([^%)]+)%)", '<a href="%2" target="_blank" rel="noopener noreferrer" class="text-primary hover:text-primary-light hover:underline">%1</a>')
  
  -- Bold: **text**
  formatted = formatted:gsub("%*%*([^%*]+)%*%*", "<strong class='font-semibold'>%1</strong>")

  -- Italic: *text*
  formatted = formatted:gsub("%*([^%*]+)%*", "<em>%1</em>")

  -- Blockquotes: lines starting with >
  -- Process line by line to handle blockquotes
  local result_parts = {}
  local in_quote = false
  local quote_lines = {}
  local skip_empty = false

  for line in (formatted .. "\n"):gmatch("([^\n]*)\n") do
    if line:match("^&gt;%s?") then
      -- This is a quote line
      local content = line:gsub("^&gt;%s?", "")
      table.insert(quote_lines, content)
      in_quote = true
    else
      -- Not a quote line
      if in_quote and #quote_lines > 0 then
        -- End of quote block, render it
        local quote_content = table.concat(quote_lines, " ")
        local quote_html = '<div class="mb-2 rounded-lg overflow-hidden border border-white/10 bg-gradient-to-r from-primary/5 to-transparent">' ..
               '<div class="flex">' ..
               '<div class="w-1 bg-primary shrink-0"></div>' ..
               '<div class="px-3 py-2 text-sm text-text-muted">' .. quote_content .. '</div>' ..
               '</div>' ..
               '</div>'
        table.insert(result_parts, quote_html)
        quote_lines = {}
        in_quote = false
        skip_empty = true
      end
      -- Skip empty lines right after a quote block
      if skip_empty and line:match("^%s*$") then
        -- Keep skipping empty lines
      else
        if #result_parts > 0 and not line:match("^%s*$") and not result_parts[#result_parts]:match("</div>$") then
          table.insert(result_parts, "<br>")
        end
        skip_empty = false
        if not line:match("^%s*$") then
          table.insert(result_parts, line)
        end
      end
    end
  end

  -- Handle trailing quote block
  if #quote_lines > 0 then
    local quote_content = table.concat(quote_lines, " ")
    local quote_html = '<div class="mb-2 rounded-lg overflow-hidden border border-white/10 bg-gradient-to-r from-primary/5 to-transparent">' ..
           '<div class="flex">' ..
           '<div class="w-1 bg-primary shrink-0"></div>' ..
           '<div class="px-3 py-2 text-sm text-text-muted">' .. quote_content .. '</div>' ..
           '</div>' ..
           '</div>'
    table.insert(result_parts, quote_html)
  end

  formatted = table.concat(result_parts, "")

  -- Restore inline code blocks
  formatted = formatted:gsub("_{{INLINE_CODE_(%d+)}}_", function(i)
    return inline_code_blocks[tonumber(i)]
  end)

  -- Restore main code blocks (replaces <br>s that might have been added around placeholders if any, but since placeholders are one line, they are safe)
  formatted = formatted:gsub("_{{CODE_BLOCK_(%d+)}}_", function(i)
    return code_blocks[tonumber(i)]
  end)
  
  return formatted
end

return M
