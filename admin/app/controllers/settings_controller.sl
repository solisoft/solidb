# Settings - admin console preferences. Currently: the theme preset
# (accent color), stored in the `admin_theme` cookie and applied by the
# layout as <body data-theme="...">.

class SettingsController < Controller
  static {
    this.layout = "application"
  }

  # GET /settings
  def index
    @title = "Settings"
    @databases = AdminContext.database_names()
    @presets = AdminTheme.presets()
    @current_theme = AdminTheme.valid(cookies["admin_theme"] ?? "")
    # Helpers cannot reach service classes, so the view reads these as locals.
    swatches = {}
    blurbs = {}
    for preset in @presets
      swatches[preset] = AdminTheme.swatch(preset)
      blurbs[preset] = AdminTheme.blurb(preset)
    end
    @theme_swatches = swatches
    @theme_blurbs = blurbs
    @fonts = AdminTheme.fonts()
    @current_font = AdminTheme.font_valid(cookies["admin_font"] ?? "")
    font_blurbs = {}
    for font in @fonts
      font_blurbs[font] = AdminTheme.font_blurb(font)
    end
    @font_blurbs = font_blurbs
  end

  # POST /settings/theme
  def update_theme
    theme = AdminTheme.valid(params["theme"] ?? "")
    set_cookie("admin_theme", theme)
    redirect(settings_path())
  end

  # POST /settings/font
  def update_font
    font = AdminTheme.font_valid(params["font"] ?? "")
    set_cookie("admin_font", font)
    redirect(settings_path())
  end
end
