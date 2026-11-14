#!/usr/bin/env python3
"""Render the share card of every blog post.

    ./tools/og/posts.py              # every post
    ./tools/og/posts.py <slug> ...   # only these

Writes public/images/blog/<slug>.png (1200x630): the post's headline beside
the first <figure class="post-figure"> of its view. The posts, their titles,
dates and tags come from BlogController#posts; the blog layout points
og:image and twitter:image at the file, and tests/share_meta_spec.sl fails
when a post has no card or no figure. Re-run after adding a post or changing
a post's first figure, and commit the PNG.

Needs a Chromium on PATH (or CHROME=...) and network access for the web fonts,
like tools/og/build.sh.
"""

import html
import os
import re
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
APP = os.path.normpath(os.path.join(HERE, "..", ".."))
CONTROLLER = os.path.join(APP, "app", "controllers", "blog_controller.sl")
VIEWS = os.path.join(APP, "app", "views", "blog")
OUT = os.path.join(APP, "public", "images", "blog")
TEMPLATE = os.path.join(HERE, "post.html")


def posts():
    """The entries of BlogController#posts, read from its literal."""
    source = open(CONTROLLER, encoding="utf-8").read()
    body = source.split("def posts", 1)[1].split("\n  end", 1)[0]
    found = []
    for block in re.split(r"\n\s*\},?\s*\n\s*\{", body):
        slug = re.search(r'"slug":\s*"([^"]+)"', block)
        if not slug:
            continue
        found.append({
            "slug": slug.group(1),
            "title": re.search(r'"title":\s*"([^"]+)"', block).group(1),
            "date": re.search(r'"display_date":\s*"([^"]+)"', block).group(1),
            "tags": re.findall(r'"([^"]+)"', re.search(r'"tags":\s*\[([^\]]*)\]', block).group(1)),
            "minutes": re.search(r'"read_minutes":\s*(\d+)', block).group(1),
        })
    return found


def first_figure(slug):
    view = open(os.path.join(VIEWS, slug + ".html.slv"), encoding="utf-8").read()
    match = re.search(r'<figure class="post-figure">\s*(<svg\b.*?</svg>)', view, re.S)
    if not match:
        sys.exit(f"error: app/views/blog/{slug}.html.slv has no <figure class=\"post-figure\"><svg>")
    svg = match.group(1)
    if "<%" in svg:
        sys.exit(f"error: the first figure of {slug} contains ERB; the card renders it as static SVG")
    return svg


def chrome():
    explicit = os.environ.get("CHROME")
    if explicit:
        return explicit
    for candidate in ("chromium", "chromium-browser", "google-chrome", "/snap/bin/chromium"):
        path = shutil.which(candidate)
        if path:
            return path
    sys.exit("error: no chromium found. Set CHROME=/path/to/chrome and re-run.")


def main(wanted):
    template = open(TEMPLATE, encoding="utf-8").read()
    selected = [p for p in posts() if not wanted or p["slug"] in wanted]
    missing = set(wanted) - {p["slug"] for p in selected}
    if missing:
        sys.exit("error: not in BlogController#posts: " + ", ".join(sorted(missing)))

    browser = chrome()
    os.makedirs(OUT, exist_ok=True)
    # Snap-confined Chromium cannot read or write /tmp or dot-directories, so
    # stage in a visible directory under $HOME (see build.sh).
    staging = tempfile.mkdtemp(prefix="soli-og-posts.", dir=os.path.expanduser("~"))
    try:
        for post in selected:
            release = next((t for t in post["tags"] if t.startswith("Release")), None)
            kicker = release or post["tags"][0]
            meta = f'{post["date"]} · {post["minutes"]} min read'
            page = (template
                    .replace("<!--KICKER-->", html.escape(kicker))
                    .replace("<!--TITLE-->", html.escape(post["title"]))
                    .replace("<!--META-->", html.escape(meta))
                    .replace("<!--FIGURE-->", first_figure(post["slug"])))
            page_path = os.path.join(staging, post["slug"] + ".html")
            shot = os.path.join(staging, post["slug"] + ".png")
            with open(page_path, "w", encoding="utf-8") as handle:
                handle.write(page)
            # --virtual-time-budget lets the web fonts land before the shutter.
            subprocess.run([browser, "--headless=new", "--disable-gpu", "--no-sandbox",
                            "--hide-scrollbars", "--force-device-scale-factor=1",
                            "--window-size=1200,630", "--virtual-time-budget=8000",
                            "--screenshot=" + shot, "file://" + page_path],
                           check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            shutil.move(shot, os.path.join(OUT, post["slug"] + ".png"))
            print(f'  wrote public/images/blog/{post["slug"]}.png')
    finally:
        shutil.rmtree(staging, ignore_errors=True)


if __name__ == "__main__":
    main(sys.argv[1:])
