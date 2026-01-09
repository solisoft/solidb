-- File type detection helper
-- Provides icons and syntax highlighting classes for various file types

local M = {}

-- File extension to type mapping
M.extensions = {
  -- Programming Languages
  lua = { icon = "fa-file-code", color = "text-blue-400", lang = "lua" },
  py = { icon = "fa-file-code", color = "text-yellow-400", lang = "python" },
  python = { icon = "fa-file-code", color = "text-yellow-400", lang = "python" },
  js = { icon = "fa-file-code", color = "text-yellow-300", lang = "javascript" },
  mjs = { icon = "fa-file-code", color = "text-yellow-300", lang = "javascript" },
  cjs = { icon = "fa-file-code", color = "text-yellow-300", lang = "javascript" },
  ts = { icon = "fa-file-code", color = "text-blue-400", lang = "typescript" },
  tsx = { icon = "fa-file-code", color = "text-blue-400", lang = "typescript" },
  jsx = { icon = "fa-file-code", color = "text-cyan-400", lang = "javascript" },
  rs = { icon = "fa-file-code", color = "text-orange-400", lang = "rust" },
  go = { icon = "fa-file-code", color = "text-cyan-400", lang = "go" },
  rb = { icon = "fa-file-code", color = "text-red-400", lang = "ruby" },
  php = { icon = "fa-file-code", color = "text-purple-400", lang = "php" },
  java = { icon = "fa-file-code", color = "text-orange-400", lang = "java" },
  kt = { icon = "fa-file-code", color = "text-purple-400", lang = "kotlin" },
  kts = { icon = "fa-file-code", color = "text-purple-400", lang = "kotlin" },
  swift = { icon = "fa-file-code", color = "text-orange-400", lang = "swift" },
  cs = { icon = "fa-file-code", color = "text-green-400", lang = "csharp" },
  cpp = { icon = "fa-file-code", color = "text-blue-400", lang = "cpp" },
  cc = { icon = "fa-file-code", color = "text-blue-400", lang = "cpp" },
  cxx = { icon = "fa-file-code", color = "text-blue-400", lang = "cpp" },
  c = { icon = "fa-file-code", color = "text-blue-300", lang = "c" },
  h = { icon = "fa-file-code", color = "text-purple-300", lang = "c" },
  hpp = { icon = "fa-file-code", color = "text-purple-300", lang = "cpp" },
  hxx = { icon = "fa-file-code", color = "text-purple-300", lang = "cpp" },
  scala = { icon = "fa-file-code", color = "text-red-400", lang = "scala" },
  clj = { icon = "fa-file-code", color = "text-green-400", lang = "clojure" },
  cljs = { icon = "fa-file-code", color = "text-green-400", lang = "clojure" },
  ex = { icon = "fa-file-code", color = "text-purple-400", lang = "elixir" },
  exs = { icon = "fa-file-code", color = "text-purple-400", lang = "elixir" },
  erl = { icon = "fa-file-code", color = "text-red-400", lang = "erlang" },
  hrl = { icon = "fa-file-code", color = "text-red-400", lang = "erlang" },
  hs = { icon = "fa-file-code", color = "text-purple-400", lang = "haskell" },
  ml = { icon = "fa-file-code", color = "text-orange-400", lang = "ocaml" },
  mli = { icon = "fa-file-code", color = "text-orange-400", lang = "ocaml" },
  fs = { icon = "fa-file-code", color = "text-blue-400", lang = "fsharp" },
  fsx = { icon = "fa-file-code", color = "text-blue-400", lang = "fsharp" },
  r = { icon = "fa-file-code", color = "text-blue-400", lang = "r" },
  R = { icon = "fa-file-code", color = "text-blue-400", lang = "r" },
  jl = { icon = "fa-file-code", color = "text-purple-400", lang = "julia" },
  nim = { icon = "fa-file-code", color = "text-yellow-400", lang = "nim" },
  zig = { icon = "fa-file-code", color = "text-orange-400", lang = "zig" },
  v = { icon = "fa-file-code", color = "text-blue-400", lang = "v" },
  d = { icon = "fa-file-code", color = "text-red-400", lang = "d" },
  dart = { icon = "fa-file-code", color = "text-cyan-400", lang = "dart" },
  groovy = { icon = "fa-file-code", color = "text-blue-400", lang = "groovy" },
  pl = { icon = "fa-file-code", color = "text-blue-400", lang = "perl" },
  pm = { icon = "fa-file-code", color = "text-blue-400", lang = "perl" },
  tcl = { icon = "fa-file-code", color = "text-orange-400", lang = "tcl" },
  vb = { icon = "fa-file-code", color = "text-blue-400", lang = "vb" },
  pas = { icon = "fa-file-code", color = "text-blue-400", lang = "pascal" },
  asm = { icon = "fa-file-code", color = "text-gray-400", lang = "asm" },
  s = { icon = "fa-file-code", color = "text-gray-400", lang = "asm" },

  -- Web
  html = { icon = "fa-file-code", color = "text-orange-400", lang = "html" },
  htm = { icon = "fa-file-code", color = "text-orange-400", lang = "html" },
  xhtml = { icon = "fa-file-code", color = "text-orange-400", lang = "html" },
  css = { icon = "fa-file-code", color = "text-blue-400", lang = "css" },
  scss = { icon = "fa-file-code", color = "text-pink-400", lang = "scss" },
  sass = { icon = "fa-file-code", color = "text-pink-400", lang = "sass" },
  less = { icon = "fa-file-code", color = "text-blue-300", lang = "less" },
  styl = { icon = "fa-file-code", color = "text-green-400", lang = "stylus" },
  vue = { icon = "fa-file-code", color = "text-green-400", lang = "vue" },
  svelte = { icon = "fa-file-code", color = "text-orange-400", lang = "svelte" },
  astro = { icon = "fa-file-code", color = "text-purple-400", lang = "astro" },

  -- Config & Data
  json = { icon = "fa-file-code", color = "text-yellow-400", lang = "json" },
  jsonc = { icon = "fa-file-code", color = "text-yellow-400", lang = "json" },
  json5 = { icon = "fa-file-code", color = "text-yellow-400", lang = "json" },
  yaml = { icon = "fa-file-code", color = "text-red-400", lang = "yaml" },
  yml = { icon = "fa-file-code", color = "text-red-400", lang = "yaml" },
  toml = { icon = "fa-file-code", color = "text-gray-400", lang = "toml" },
  xml = { icon = "fa-file-code", color = "text-orange-400", lang = "xml" },
  xsl = { icon = "fa-file-code", color = "text-orange-400", lang = "xml" },
  xslt = { icon = "fa-file-code", color = "text-orange-400", lang = "xml" },
  ini = { icon = "fa-file-code", color = "text-gray-400", lang = "ini" },
  cfg = { icon = "fa-file-code", color = "text-gray-400", lang = "ini" },
  conf = { icon = "fa-file-code", color = "text-gray-400", lang = "ini" },
  env = { icon = "fa-file-code", color = "text-yellow-400", lang = "dotenv" },
  properties = { icon = "fa-file-code", color = "text-gray-400", lang = "properties" },
  csv = { icon = "fa-file-csv", color = "text-green-400", lang = "csv" },
  tsv = { icon = "fa-file-csv", color = "text-green-400", lang = "csv" },

  -- Shell & Scripts
  sh = { icon = "fa-terminal", color = "text-green-400", lang = "bash" },
  bash = { icon = "fa-terminal", color = "text-green-400", lang = "bash" },
  zsh = { icon = "fa-terminal", color = "text-green-400", lang = "bash" },
  fish = { icon = "fa-terminal", color = "text-green-400", lang = "fish" },
  ps1 = { icon = "fa-terminal", color = "text-blue-400", lang = "powershell" },
  psm1 = { icon = "fa-terminal", color = "text-blue-400", lang = "powershell" },
  bat = { icon = "fa-terminal", color = "text-green-400", lang = "batch" },
  cmd = { icon = "fa-terminal", color = "text-green-400", lang = "batch" },

  -- Documentation
  md = { icon = "fa-file-alt", color = "text-blue-400", lang = "markdown" },
  markdown = { icon = "fa-file-alt", color = "text-blue-400", lang = "markdown" },
  mdx = { icon = "fa-file-alt", color = "text-yellow-400", lang = "mdx" },
  rst = { icon = "fa-file-alt", color = "text-gray-400", lang = "rst" },
  txt = { icon = "fa-file-alt", color = "text-gray-400", lang = "text" },
  text = { icon = "fa-file-alt", color = "text-gray-400", lang = "text" },
  tex = { icon = "fa-file-alt", color = "text-green-400", lang = "latex" },
  latex = { icon = "fa-file-alt", color = "text-green-400", lang = "latex" },
  org = { icon = "fa-file-alt", color = "text-cyan-400", lang = "org" },
  adoc = { icon = "fa-file-alt", color = "text-red-400", lang = "asciidoc" },
  asciidoc = { icon = "fa-file-alt", color = "text-red-400", lang = "asciidoc" },

  -- Database
  sql = { icon = "fa-database", color = "text-blue-400", lang = "sql" },
  mysql = { icon = "fa-database", color = "text-blue-400", lang = "sql" },
  pgsql = { icon = "fa-database", color = "text-blue-400", lang = "sql" },
  sqlite = { icon = "fa-database", color = "text-blue-400", lang = "sql" },
  prisma = { icon = "fa-database", color = "text-cyan-400", lang = "prisma" },
  graphql = { icon = "fa-project-diagram", color = "text-pink-400", lang = "graphql" },
  gql = { icon = "fa-project-diagram", color = "text-pink-400", lang = "graphql" },

  -- DevOps & Infra
  dockerfile = { icon = "fa-docker", color = "text-blue-400", lang = "dockerfile" },
  docker = { icon = "fa-docker", color = "text-blue-400", lang = "dockerfile" },
  tf = { icon = "fa-cloud", color = "text-purple-400", lang = "hcl" },
  hcl = { icon = "fa-cloud", color = "text-purple-400", lang = "hcl" },
  vagrant = { icon = "fa-cloud", color = "text-blue-400", lang = "ruby" },
  ansible = { icon = "fa-cogs", color = "text-red-400", lang = "yaml" },
  helm = { icon = "fa-dharmachakra", color = "text-blue-400", lang = "yaml" },

  -- Build & Package
  makefile = { icon = "fa-cogs", color = "text-orange-400", lang = "makefile" },
  cmake = { icon = "fa-cogs", color = "text-blue-400", lang = "cmake" },
  gradle = { icon = "fa-cogs", color = "text-green-400", lang = "groovy" },
  maven = { icon = "fa-cogs", color = "text-red-400", lang = "xml" },
  rake = { icon = "fa-cogs", color = "text-red-400", lang = "ruby" },

  -- Images
  png = { icon = "fa-file-image", color = "text-purple-400", lang = nil, binary = true },
  jpg = { icon = "fa-file-image", color = "text-purple-400", lang = nil, binary = true },
  jpeg = { icon = "fa-file-image", color = "text-purple-400", lang = nil, binary = true },
  gif = { icon = "fa-file-image", color = "text-purple-400", lang = nil, binary = true },
  svg = { icon = "fa-file-image", color = "text-orange-400", lang = "xml" },
  webp = { icon = "fa-file-image", color = "text-purple-400", lang = nil, binary = true },
  ico = { icon = "fa-file-image", color = "text-purple-400", lang = nil, binary = true },
  bmp = { icon = "fa-file-image", color = "text-purple-400", lang = nil, binary = true },
  tiff = { icon = "fa-file-image", color = "text-purple-400", lang = nil, binary = true },
  psd = { icon = "fa-file-image", color = "text-blue-400", lang = nil, binary = true },
  ai = { icon = "fa-file-image", color = "text-orange-400", lang = nil, binary = true },

  -- Audio/Video
  mp3 = { icon = "fa-file-audio", color = "text-pink-400", lang = nil, binary = true },
  wav = { icon = "fa-file-audio", color = "text-pink-400", lang = nil, binary = true },
  ogg = { icon = "fa-file-audio", color = "text-pink-400", lang = nil, binary = true },
  flac = { icon = "fa-file-audio", color = "text-pink-400", lang = nil, binary = true },
  mp4 = { icon = "fa-file-video", color = "text-red-400", lang = nil, binary = true },
  webm = { icon = "fa-file-video", color = "text-red-400", lang = nil, binary = true },
  mkv = { icon = "fa-file-video", color = "text-red-400", lang = nil, binary = true },
  avi = { icon = "fa-file-video", color = "text-red-400", lang = nil, binary = true },
  mov = { icon = "fa-file-video", color = "text-red-400", lang = nil, binary = true },

  -- Archives
  zip = { icon = "fa-file-archive", color = "text-yellow-400", lang = nil, binary = true },
  tar = { icon = "fa-file-archive", color = "text-yellow-400", lang = nil, binary = true },
  gz = { icon = "fa-file-archive", color = "text-yellow-400", lang = nil, binary = true },
  bz2 = { icon = "fa-file-archive", color = "text-yellow-400", lang = nil, binary = true },
  xz = { icon = "fa-file-archive", color = "text-yellow-400", lang = nil, binary = true },
  ["7z"] = { icon = "fa-file-archive", color = "text-yellow-400", lang = nil, binary = true },
  rar = { icon = "fa-file-archive", color = "text-yellow-400", lang = nil, binary = true },

  -- Documents
  pdf = { icon = "fa-file-pdf", color = "text-red-400", lang = nil, binary = true },
  doc = { icon = "fa-file-word", color = "text-blue-400", lang = nil, binary = true },
  docx = { icon = "fa-file-word", color = "text-blue-400", lang = nil, binary = true },
  xls = { icon = "fa-file-excel", color = "text-green-400", lang = nil, binary = true },
  xlsx = { icon = "fa-file-excel", color = "text-green-400", lang = nil, binary = true },
  ppt = { icon = "fa-file-powerpoint", color = "text-orange-400", lang = nil, binary = true },
  pptx = { icon = "fa-file-powerpoint", color = "text-orange-400", lang = nil, binary = true },
  odt = { icon = "fa-file-alt", color = "text-blue-400", lang = nil, binary = true },
  ods = { icon = "fa-file-alt", color = "text-green-400", lang = nil, binary = true },
  odp = { icon = "fa-file-alt", color = "text-orange-400", lang = nil, binary = true },

  -- Fonts
  ttf = { icon = "fa-font", color = "text-gray-400", lang = nil, binary = true },
  otf = { icon = "fa-font", color = "text-gray-400", lang = nil, binary = true },
  woff = { icon = "fa-font", color = "text-gray-400", lang = nil, binary = true },
  woff2 = { icon = "fa-font", color = "text-gray-400", lang = nil, binary = true },
  eot = { icon = "fa-font", color = "text-gray-400", lang = nil, binary = true },

  -- Binaries & Executables
  exe = { icon = "fa-cog", color = "text-gray-400", lang = nil, binary = true },
  dll = { icon = "fa-cog", color = "text-gray-400", lang = nil, binary = true },
  so = { icon = "fa-cog", color = "text-gray-400", lang = nil, binary = true },
  dylib = { icon = "fa-cog", color = "text-gray-400", lang = nil, binary = true },
  a = { icon = "fa-cog", color = "text-gray-400", lang = nil, binary = true },
  o = { icon = "fa-cog", color = "text-gray-400", lang = nil, binary = true },
  class = { icon = "fa-cog", color = "text-orange-400", lang = nil, binary = true },
  pyc = { icon = "fa-cog", color = "text-yellow-400", lang = nil, binary = true },
  wasm = { icon = "fa-cog", color = "text-purple-400", lang = nil, binary = true },

  -- Misc
  lock = { icon = "fa-lock", color = "text-yellow-400", lang = "json" },
  log = { icon = "fa-file-alt", color = "text-gray-400", lang = "log" },
  diff = { icon = "fa-file-code", color = "text-green-400", lang = "diff" },
  patch = { icon = "fa-file-code", color = "text-green-400", lang = "diff" },
  gitignore = { icon = "fa-git-alt", color = "text-orange-400", lang = "gitignore" },
  gitattributes = { icon = "fa-git-alt", color = "text-orange-400", lang = "gitattributes" },
  gitmodules = { icon = "fa-git-alt", color = "text-orange-400", lang = "gitconfig" },
  editorconfig = { icon = "fa-cog", color = "text-gray-400", lang = "editorconfig" },
  prettierrc = { icon = "fa-cog", color = "text-pink-400", lang = "json" },
  eslintrc = { icon = "fa-cog", color = "text-purple-400", lang = "json" },
  babelrc = { icon = "fa-cog", color = "text-yellow-400", lang = "json" },
  nvmrc = { icon = "fa-cog", color = "text-green-400", lang = "text" },
  npmrc = { icon = "fa-cog", color = "text-red-400", lang = "ini" },
}

-- Special filenames (no extension or special names)
M.filenames = {
  ["Makefile"] = { icon = "fa-cogs", color = "text-orange-400", lang = "makefile" },
  ["makefile"] = { icon = "fa-cogs", color = "text-orange-400", lang = "makefile" },
  ["GNUmakefile"] = { icon = "fa-cogs", color = "text-orange-400", lang = "makefile" },
  ["CMakeLists.txt"] = { icon = "fa-cogs", color = "text-blue-400", lang = "cmake" },
  ["Dockerfile"] = { icon = "fa-docker", color = "text-blue-400", lang = "dockerfile" },
  ["docker-compose.yml"] = { icon = "fa-docker", color = "text-blue-400", lang = "yaml" },
  ["docker-compose.yaml"] = { icon = "fa-docker", color = "text-blue-400", lang = "yaml" },
  [".dockerignore"] = { icon = "fa-docker", color = "text-blue-400", lang = "gitignore" },
  [".gitignore"] = { icon = "fa-git-alt", color = "text-orange-400", lang = "gitignore" },
  [".gitattributes"] = { icon = "fa-git-alt", color = "text-orange-400", lang = "gitattributes" },
  [".gitmodules"] = { icon = "fa-git-alt", color = "text-orange-400", lang = "gitconfig" },
  [".editorconfig"] = { icon = "fa-cog", color = "text-gray-400", lang = "editorconfig" },
  [".prettierrc"] = { icon = "fa-cog", color = "text-pink-400", lang = "json" },
  [".prettierrc.json"] = { icon = "fa-cog", color = "text-pink-400", lang = "json" },
  [".eslintrc"] = { icon = "fa-cog", color = "text-purple-400", lang = "json" },
  [".eslintrc.json"] = { icon = "fa-cog", color = "text-purple-400", lang = "json" },
  [".babelrc"] = { icon = "fa-cog", color = "text-yellow-400", lang = "json" },
  [".env"] = { icon = "fa-cog", color = "text-yellow-400", lang = "dotenv" },
  [".env.local"] = { icon = "fa-cog", color = "text-yellow-400", lang = "dotenv" },
  [".env.development"] = { icon = "fa-cog", color = "text-yellow-400", lang = "dotenv" },
  [".env.production"] = { icon = "fa-cog", color = "text-yellow-400", lang = "dotenv" },
  [".env.test"] = { icon = "fa-cog", color = "text-yellow-400", lang = "dotenv" },
  [".nvmrc"] = { icon = "fa-cog", color = "text-green-400", lang = "text" },
  [".npmrc"] = { icon = "fa-cog", color = "text-red-400", lang = "ini" },
  [".yarnrc"] = { icon = "fa-cog", color = "text-blue-400", lang = "yaml" },
  ["package.json"] = { icon = "fa-npm", color = "text-red-400", lang = "json" },
  ["package-lock.json"] = { icon = "fa-npm", color = "text-red-400", lang = "json" },
  ["yarn.lock"] = { icon = "fa-lock", color = "text-blue-400", lang = "yaml" },
  ["pnpm-lock.yaml"] = { icon = "fa-lock", color = "text-orange-400", lang = "yaml" },
  ["Cargo.toml"] = { icon = "fa-cube", color = "text-orange-400", lang = "toml" },
  ["Cargo.lock"] = { icon = "fa-lock", color = "text-orange-400", lang = "toml" },
  ["Gemfile"] = { icon = "fa-gem", color = "text-red-400", lang = "ruby" },
  ["Gemfile.lock"] = { icon = "fa-lock", color = "text-red-400", lang = "text" },
  ["requirements.txt"] = { icon = "fa-python", color = "text-yellow-400", lang = "text" },
  ["setup.py"] = { icon = "fa-python", color = "text-yellow-400", lang = "python" },
  ["pyproject.toml"] = { icon = "fa-python", color = "text-yellow-400", lang = "toml" },
  ["Pipfile"] = { icon = "fa-python", color = "text-yellow-400", lang = "toml" },
  ["Pipfile.lock"] = { icon = "fa-lock", color = "text-yellow-400", lang = "json" },
  ["go.mod"] = { icon = "fa-cube", color = "text-cyan-400", lang = "go" },
  ["go.sum"] = { icon = "fa-lock", color = "text-cyan-400", lang = "text" },
  ["composer.json"] = { icon = "fa-cube", color = "text-yellow-400", lang = "json" },
  ["composer.lock"] = { icon = "fa-lock", color = "text-yellow-400", lang = "json" },
  ["build.gradle"] = { icon = "fa-cogs", color = "text-green-400", lang = "groovy" },
  ["settings.gradle"] = { icon = "fa-cogs", color = "text-green-400", lang = "groovy" },
  ["pom.xml"] = { icon = "fa-cogs", color = "text-red-400", lang = "xml" },
  ["mix.exs"] = { icon = "fa-cube", color = "text-purple-400", lang = "elixir" },
  ["mix.lock"] = { icon = "fa-lock", color = "text-purple-400", lang = "elixir" },
  ["pubspec.yaml"] = { icon = "fa-cube", color = "text-cyan-400", lang = "yaml" },
  ["pubspec.lock"] = { icon = "fa-lock", color = "text-cyan-400", lang = "yaml" },
  ["README"] = { icon = "fa-book", color = "text-blue-400", lang = "text" },
  ["README.md"] = { icon = "fa-book", color = "text-blue-400", lang = "markdown" },
  ["README.txt"] = { icon = "fa-book", color = "text-blue-400", lang = "text" },
  ["LICENSE"] = { icon = "fa-balance-scale", color = "text-yellow-400", lang = "text" },
  ["LICENSE.md"] = { icon = "fa-balance-scale", color = "text-yellow-400", lang = "markdown" },
  ["LICENSE.txt"] = { icon = "fa-balance-scale", color = "text-yellow-400", lang = "text" },
  ["CHANGELOG"] = { icon = "fa-list", color = "text-green-400", lang = "text" },
  ["CHANGELOG.md"] = { icon = "fa-list", color = "text-green-400", lang = "markdown" },
  ["CONTRIBUTING.md"] = { icon = "fa-hands-helping", color = "text-purple-400", lang = "markdown" },
  ["CODE_OF_CONDUCT.md"] = { icon = "fa-gavel", color = "text-red-400", lang = "markdown" },
  ["CLAUDE.md"] = { icon = "fa-robot", color = "text-orange-400", lang = "markdown" },
  ["Vagrantfile"] = { icon = "fa-cube", color = "text-blue-400", lang = "ruby" },
  ["Rakefile"] = { icon = "fa-cogs", color = "text-red-400", lang = "ruby" },
  ["Procfile"] = { icon = "fa-server", color = "text-purple-400", lang = "text" },
  ["Brewfile"] = { icon = "fa-beer", color = "text-yellow-400", lang = "ruby" },
  [".htaccess"] = { icon = "fa-server", color = "text-green-400", lang = "apache" },
  ["nginx.conf"] = { icon = "fa-server", color = "text-green-400", lang = "nginx" },
  ["tsconfig.json"] = { icon = "fa-cog", color = "text-blue-400", lang = "json" },
  ["jsconfig.json"] = { icon = "fa-cog", color = "text-yellow-400", lang = "json" },
  ["webpack.config.js"] = { icon = "fa-cube", color = "text-blue-400", lang = "javascript" },
  ["vite.config.js"] = { icon = "fa-bolt", color = "text-purple-400", lang = "javascript" },
  ["vite.config.ts"] = { icon = "fa-bolt", color = "text-purple-400", lang = "typescript" },
  ["rollup.config.js"] = { icon = "fa-cube", color = "text-red-400", lang = "javascript" },
  ["tailwind.config.js"] = { icon = "fa-wind", color = "text-cyan-400", lang = "javascript" },
  ["postcss.config.js"] = { icon = "fa-cog", color = "text-red-400", lang = "javascript" },
  ["jest.config.js"] = { icon = "fa-vial", color = "text-green-400", lang = "javascript" },
  ["vitest.config.ts"] = { icon = "fa-vial", color = "text-green-400", lang = "typescript" },
}

-- Default for unknown files
M.default = { icon = "fa-file", color = "text-text-dim", lang = nil }

-- Get file info by filename
function M.get_info(filename)
  -- Check special filenames first
  if M.filenames[filename] then
    return M.filenames[filename]
  end

  -- Get extension
  local ext = filename:match("%.([^%.]+)$")
  if ext then
    ext = ext:lower()
    if M.extensions[ext] then
      return M.extensions[ext]
    end
  end

  -- Check for dotfiles that might have extensions
  if filename:sub(1, 1) == "." then
    local dotfile_ext = filename:sub(2):match("([^%.]+)$")
    if dotfile_ext and M.extensions[dotfile_ext] then
      return M.extensions[dotfile_ext]
    end
  end

  return M.default
end

-- Get icon class for a file
function M.get_icon(filename)
  local info = M.get_info(filename)
  return "fas " .. info.icon
end

-- Get color class for a file
function M.get_color(filename)
  local info = M.get_info(filename)
  return info.color
end

-- Get language for syntax highlighting
function M.get_language(filename)
  local info = M.get_info(filename)
  return info.lang
end

-- Check if file is binary
function M.is_binary(filename)
  local info = M.get_info(filename)
  return info.binary == true
end

-- Check if file is an image
function M.is_image(filename)
  local ext = filename:match("%.([^%.]+)$")
  if ext then
    ext = ext:lower()
    return ext:match("^(png|jpg|jpeg|gif|svg|webp|ico|bmp|tiff)$") ~= nil
  end
  return false
end

return M
