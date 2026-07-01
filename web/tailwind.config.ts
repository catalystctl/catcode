import type { Config } from "tailwindcss";

const config: Config = {
  content: [
    "./src/**/*.{ts,tsx,mdx}",
  ],
  theme: {
    extend: {
      fontFamily: {
        sans: ["var(--font-sans)", "ui-sans-serif", "system-ui", "sans-serif"],
        mono: ["var(--font-mono)", "ui-monospace", "SFMono-Regular", "Menlo", "monospace"],
      },
      colors: {
        // Refined dark surface scale (zinc-tuned, slightly cooler)
        ink: {
          950: "#08080a",
          925: "#0c0c10",
          900: "#101015",
          850: "#15151c",
          800: "#1b1b23",
          750: "#22222c",
          700: "#2a2a35",
          600: "#3a3a47",
          500: "#52525f",
          400: "#6e6e7e",
          300: "#9a9aa8",
          200: "#c4c4cf",
          100: "#e6e6ec",
        },
        accent: {
          DEFAULT: "#8b7cff",
          soft: "#a99cff",
          deep: "#6d5ce6",
        },
      },
      keyframes: {
        "fade-in": {
          "0%": { opacity: "0", transform: "translateY(4px)" },
          "100%": { opacity: "1", transform: "translateY(0)" },
        },
        shimmer: {
          "0%": { backgroundPosition: "-200% 0" },
          "100%": { backgroundPosition: "200% 0" },
        },
        "caret-blink": {
          "0%,70%,100%": { opacity: "1" },
          "20%,50%": { opacity: "0" },
        },
        "pulse-ring": {
          "0%": { transform: "scale(0.8)", opacity: "0.7" },
          "100%": { transform: "scale(2)", opacity: "0" },
        },
      },
      animation: {
        "fade-in": "fade-in 0.18s ease-out",
        shimmer: "shimmer 2.5s linear infinite",
        "caret-blink": "caret-blink 1.1s steps(1) infinite",
        "pulse-ring": "pulse-ring 1.4s ease-out infinite",
      },
      boxShadow: {
        glow: "0 0 0 1px rgba(139,124,255,0.25), 0 0 24px -6px rgba(139,124,255,0.45)",
      },
    },
  },
  plugins: [],
};

export default config;
