/** @type {import('tailwindcss').Config} */
// UnknownChat foundations: every value mirrors a token in src/index.css.
export default {
  content: ['./index.html', './src/**/*.{ts,tsx}'],
  theme: {
    extend: {
      colors: {
        chassis: 'var(--bg-chassis)',
        panel: 'var(--bg-panel)',
        stage: 'var(--bg-stage)',
        raised: 'var(--bg-raised)',
        field: 'var(--bg-field)',
        overlay: 'var(--bg-overlay)',
        selected: 'var(--bg-selected)',
        code: 'var(--bg-code)',
        line: {
          subtle: 'var(--line-subtle)',
          DEFAULT: 'var(--line-default)',
          strong: 'var(--line-strong)',
          heavy: 'var(--line-heavy)',
        },
        ink: {
          1: 'var(--ink-1)',
          body: 'var(--ink-body)',
          2: 'var(--ink-2)',
          3: 'var(--ink-3)',
          off: 'var(--ink-off)',
        },
        uv: {
          100: 'var(--uv-100)',
          200: 'var(--uv-200)',
          300: 'var(--uv-300)',
          400: 'var(--uv-400)',
          500: 'var(--uv-500)',
          600: 'var(--uv-600)',
          700: 'var(--uv-700)',
          950: 'var(--uv-950)',
        },
        mint: { DEFAULT: 'var(--mint)', text: 'var(--mint-text)' },
        amber: { DEFAULT: 'var(--amber)' },
        rose: { DEFAULT: 'var(--rose)', text: 'var(--rose-text)' },
        sky: { DEFAULT: 'var(--sky)' },
      },
      fontFamily: {
        display: ['"Chakra Petch"', '"Instrument Sans"', 'sans-serif'],
        sans: ['"Instrument Sans"', 'system-ui', '-apple-system', '"Segoe UI"', 'sans-serif'],
        mono: ['"JetBrains Mono"', 'ui-monospace', 'monospace'],
      },
      borderRadius: {
        xs: '3px',
        sm: '5px',
        md: '8px',
        lg: '12px',
      },
    },
  },
  plugins: [],
};
