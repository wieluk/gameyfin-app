/** Values are `<r> <g> <b>` channels from `styles.css`, which is what makes `/50` opacity work. */
const semantic = (name) => `rgb(var(--gf-${name}) / <alpha-value>)`;

/** @type {import('tailwindcss').Config} */
export default {
  content: ["./index.html", "./src/**/*.{js,ts,jsx,tsx}"],
  darkMode: "class",
  theme: {
    extend: {
      colors: {
        background: semantic("background"),
        foreground: semantic("foreground"),
        content1: semantic("content1"),
        content2: semantic("content2"),
        default: {
          100: semantic("default-100"),
          200: semantic("default-200"),
          300: semantic("default-300"),
        },
        primary: {
          DEFAULT: semantic("primary"),
          600: semantic("primary-600"),
          foreground: semantic("primary-foreground"),
        },
        danger: { DEFAULT: semantic("danger"), 600: semantic("danger-600") },
        warning: { DEFAULT: semantic("warning"), 600: semantic("warning-600") },
        success: { DEFAULT: semantic("success"), 600: semantic("success-600") },
      },
    },
  },
  plugins: [],
};
