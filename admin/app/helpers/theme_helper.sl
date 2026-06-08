# Current theme preset for the layout's <body data-theme="...">.
# Helpers run in template scope where service classes (AdminTheme) are not
# visible, so the whitelist is repeated here - keep it in sync with
# app/services/admin_theme.sl. Whitelisting matters: the cookie is
# client-controlled and the value lands in an attribute via raw output.

def admin_theme()
  theme = (cookies["admin_theme"] rescue nil) ?? "solidb"
  presets = ["solidb", "arangodb", "violet", "amber", "rose", "sky",
             "solidb-light", "arangodb-light", "violet-light", "amber-light", "rose-light", "sky-light"]
  return presets.includes?(theme) ? theme : "solidb"
end

# Current font preset (cookie: admin_font) - keep the list in sync with
# AdminTheme.fonts().
def admin_font()
  font = (cookies["admin_font"] rescue nil) ?? "grotesk"
  fonts = ["grotesk", "inter", "ibm-plex", "source", "system"]
  return fonts.includes?(font) ? font : "grotesk"
end

# Google Fonts stylesheet for the active font preset only ("" for system
# fonts - nothing to download). Values are fixed strings keyed by the
# whitelisted preset, so the URL is attr-safe.
def admin_font_link()
  base = "https://fonts.googleapis.com/css2?family="
  weights = ":wght@300;400;500;600;700&family="
  mono_weights = ":wght@400;500;600&display=swap"
  links = {
    "grotesk":  base + "Space+Grotesk" + weights + "JetBrains+Mono" + mono_weights,
    "inter":    base + "Inter" + weights + "JetBrains+Mono" + mono_weights,
    "ibm-plex": base + "IBM+Plex+Sans" + weights + "IBM+Plex+Mono" + mono_weights,
    "source":   base + "Source+Sans+3" + weights + "Source+Code+Pro" + mono_weights,
    "system":   ""
  }
  return links[admin_font()] ?? links["grotesk"]
end
