# SoliDB Investor Pitch Deck

A Reveal.js-based pitch deck for SoliDB's seed funding round.

## Quick Start

Open `index.html` in any modern browser to view the presentation.

Or serve it locally:

```bash
cd pitch
python3 -m http.server 8000
# Then open http://localhost:8000
```

## Navigation

- **Arrow keys** or **space** to navigate slides
- **F** for fullscreen
- **O** for overview
- **S** for speaker notes

## Structure

```
pitch/
├── index.html          # Main presentation
└── slides/
    ├── 01-cover.md
    ├── 02-problem.md
    ├── 03-solution.md
    ├── 04-product.md
    ├── 05-traction.md
    ├── 06-business-model.md
    ├── 07-market.md
    ├── 08-competition.md
    ├── 09-gtm.md
    ├── 10-team.md
    ├── 11-use-of-funds.md
    └── 12-cta.md
```

## Customization

Edit the markdown files in `slides/` to update content.
Colors and styles can be modified in `index.html`'s `<style>` section.
