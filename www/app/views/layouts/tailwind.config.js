/** @type {import('tailwindcss').Config} */
module.exports = {
  content: ["./app/**/*.{html,js,riot,etlua}"],
  // Theme configuration is now handled in css_tw_src.css using Tailwind v4 CSS variables
  theme: {
    extend: {},
  },
  plugins: [],
};

