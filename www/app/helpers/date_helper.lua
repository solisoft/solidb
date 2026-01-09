local DateHelper = {}

function DateHelper.time_ago_in_words(time)
  if not time then return "" end
  local now = os.time()
  local diff = now - time
  
  if diff < 60 then
    return diff .. " seconds ago"
  elseif diff < 3600 then
    local mins = math.floor(diff / 60)
    return mins .. (mins == 1 and " minute ago" or " minutes ago")
  elseif diff < 86400 then
    local hours = math.floor(diff / 3600)
    return hours .. (hours == 1 and " hour ago" or " hours ago")
  elseif diff < 2592000 then -- 30 days
    local days = math.floor(diff / 86400)
    return days .. (days == 1 and " day ago" or " days ago")
  elseif diff < 31536000 then -- 365 days
    local months = math.floor(diff / 2592000)
    return months .. (months == 1 and " month ago" or " months ago")
  else
    local years = math.floor(diff / 31536000)
    return years .. (years == 1 and " year ago" or " years ago")
  end
end

return DateHelper
