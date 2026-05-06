/** @type {import('tailwindcss').Config} */
module.exports = {
  darkMode: "class",
  content: ["./src/web/static/**/*.html", "./src/web/static/**/*.js", "./src/web/static/js/**/*.js"],
  theme: {
    extend: {
      colors: {
        ink: "#172033",
        panel: "#ffffff",
        line: "#d9e1ea",
        quiet: "#667085",
        accent: "#0f766e",
        danger: "#b42318",
        warn: "#b54708",
      },
    },
  },
  plugins: [],
};
