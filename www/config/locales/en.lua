-- English translations
-- config/locales/en.lua

return {
  -- Common
  hello = "Hello",
  welcome = "Welcome, %s!",
  goodbye = "Goodbye",
  
  -- Navigation
  nav = {
    home = "Home",
    about = "About",
    docs = "Documentation",
    login = "Login",
    logout = "Logout",
    signup = "Sign Up"
  },
  
  -- Actions
  actions = {
    save = "Save",
    cancel = "Cancel",
    delete = "Delete",
    edit = "Edit",
    create = "Create",
    update = "Update",
    search = "Search",
    back = "Back",
    next = "Next",
    previous = "Previous"
  },
  
  -- Messages
  messages = {
    success = "Operation completed successfully",
    error = "An error occurred",
    not_found = "Not found",
    unauthorized = "You are not authorized to perform this action",
    confirm_delete = "Are you sure you want to delete this?"
  },
  
  -- Model validation errors
  models = {
    errors = {
      presence = "can't be blank",
      format = "is invalid",
      acceptance = "must be accepted",
      inclusion = "is not included in the list",
      exclusion = "is reserved",
      comparaison = "doesn't match",
      numericality = {
        valid_number = "is not a valid number",
        valid_integer = "must be an integer"
      },
      length = {
        eq = "must be exactly %d characters",
        between = "must be between %d and %d characters",
        minimum = "must be at least %d characters",
        maximum = "must be at most %d characters"
      }
    }
  },
  
  -- Date and time formats
  date = {
    formats = {
      default = "%Y-%m-%d",
      short = "%b %d",
      long = "%B %d, %Y"
    },
    day_names = {"Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"},
    month_names = {"January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"}
  },
  
  time = {
    formats = {
      default = "%H:%M",
      short = "%H:%M",
      long = "%H:%M:%S"
    }
  },
  
  datetime = {
    formats = {
      default = "%Y-%m-%d %H:%M",
      short = "%b %d, %H:%M",
      long = "%B %d, %Y at %H:%M"
    }
  },
  
  -- Relative time
  relative_time = {
    now = "just now",
    seconds = "%d seconds ago",
    minute = "1 minute ago",
    minutes = "%d minutes ago",
    hour = "1 hour ago",
    hours = "%d hours ago",
    day = "yesterday",
    days = "%d days ago",
    week = "1 week ago",
    weeks = "%d weeks ago",
    month = "1 month ago",
    months = "%d months ago",
    year = "1 year ago",
    years = "%d years ago"
  },
  
  -- Pagination
  pagination = {
    previous = "Previous",
    next = "Next",
    page = "Page %d of %d"
  }
}
