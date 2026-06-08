# Admin theme presets. The chosen preset lives in the `admin_theme` cookie
# and is applied as <body data-theme="..."> - the CSS variable sets in
# app/assets/css/application.css do the rest. Values are whitelisted before
# touching markup: the cookie is client-controlled.

class AdminTheme
  static def presets()
    return ["solidb", "arangodb", "violet", "amber", "rose", "sky",
            "solidb-light", "arangodb-light", "violet-light", "amber-light", "rose-light", "sky-light"]
  end

  static def valid(preset)
    return AdminTheme.presets().includes?(preset) ? preset : "solidb"
  end

  # Swatch triple per preset (accent 400/500 + surface 900), for the Settings
  # page preview cards - independent of the active theme.
  static def swatch(preset)
    swatches = {
      "solidb":         ["#2dd4bf", "#14b8a6", "#18181b"],
      "arangodb":       ["#acd84b", "#94c83d", "#0f172a"],
      "violet":         ["#a78bfa", "#8b5cf6", "#181423"],
      "amber":          ["#fbbf24", "#f59e0b", "#1c1917"],
      "rose":           ["#fb7185", "#f43f5e", "#1f1114"],
      "sky":            ["#38bdf8", "#0ea5e9", "#111827"],
      "solidb-light":   ["#0d9488", "#14b8a6", "#f4f4f5"],
      "arangodb-light": ["#76a52c", "#94c83d", "#f1f5f9"],
      "violet-light":   ["#7c3aed", "#8b5cf6", "#f0eef8"],
      "amber-light":    ["#d97706", "#f59e0b", "#f5f5f4"],
      "rose-light":     ["#e11d48", "#f43f5e", "#f3eff0"],
      "sky-light":      ["#0284c7", "#0ea5e9", "#f3f4f6"]
    }
    return swatches[preset] ?? swatches["solidb"]
  end

  # --- font presets (cookie: admin_font, applied as data-font on <body>) ----

  static def fonts()
    return ["grotesk", "inter", "ibm-plex", "source", "system"]
  end

  static def font_valid(font)
    return AdminTheme.fonts().includes?(font) ? font : "grotesk"
  end

  static def font_blurb(font)
    blurbs = {
      "grotesk":  "Space Grotesk + JetBrains Mono - the default",
      "inter":    "Inter + JetBrains Mono - neutral, reads well on light themes",
      "ibm-plex": "IBM Plex Sans + IBM Plex Mono - engineering classic",
      "source":   "Source Sans 3 + Source Code Pro - Adobe's workhorse pair",
      "system":   "your OS fonts - zero download"
    }
    return blurbs[font] ?? ""
  end

  # Human blurb per preset for the Settings page.
  static def blurb(preset)
    blurbs = {
      "solidb":         "terminal teal on zinc - the default ops-console look",
      "arangodb":       "avocado green on slate blue, for the homesick",
      "violet":         "deep purple accents on purple-tinted dark",
      "amber":          "warm amber accents on stone",
      "rose":           "rose red accents on warm dark",
      "sky":            "sky blue accents on cool gray",
      "solidb-light":   "teal on light zinc",
      "arangodb-light": "avocado green on light slate - like the real thing",
      "violet-light":   "purple on light lavender",
      "amber-light":    "amber on warm paper",
      "rose-light":     "rose on warm white",
      "sky-light":      "sky blue on cool white"
    }
    return blurbs[preset] ?? ""
  end
end
