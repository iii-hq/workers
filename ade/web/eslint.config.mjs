/* The design-system gate (`pnpm lint:design`, run by CI's ade-web job).
 *
 * ESLint here runs only @shadcn/lint's rules; Biome still owns formatting
 * and the general lint. The theme is `src/index.css` (found through
 * components.json), so a colour, size or radius is "ours" only when that
 * file declares it as a token.
 *
 * Code written before the gate is recorded in eslint-suppressions.json: a
 * file may keep its recorded count per rule but never grow it. Fix a legacy
 * finding, then run `pnpm lint:design:prune` so the file shrinks with you.
 * Never run `--suppress-all` to get a red build green. */
import { readdirSync, readFileSync } from 'node:fs'
import { join } from 'node:path'
import { plugin as shadcn } from '@shadcn/lint'
import tsParser from '@typescript-eslint/parser'
import { defineConfig } from 'eslint/config'

const here = import.meta.dirname

/* Class names our own stylesheets define. Most are imported from TS rather
   than through index.css, so the linter's theme graph cannot see them;
   reading the selectors keeps a typo in a recipe name an error. */
function stylesheetClasses(dir) {
  const out = new Set()
  for (const entry of readdirSync(dir, {
    recursive: true,
    withFileTypes: true,
  })) {
    if (!entry.isFile() || !entry.name.endsWith('.css')) continue
    const css = readFileSync(join(entry.parentPath, entry.name), 'utf8')
      .replace(/\/\*[\s\S]*?\*\//g, '')
      .replace(/"(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*'/g, '""')
    for (const m of css.matchAll(/\.(-?[_a-zA-Z][\w-]*)/g)) out.add(m[1])
  }
  return out
}

const ownClasses = [
  ...stylesheetClasses(join(here, 'src')),
  ...stylesheetClasses(join(here, '../../packages/traces-ui/src')),
]

/* Hook classes that carry no CSS of their own: tour anchors, Prism's
   grammar classes, and markers read by tests and selectors. */
const hookClasses = [
  'learn-iii-*',
  'language-*',
  'prism-code',
  'composer-project-context',
  'workspace-tab-strip',
  'wk-badge',
  'wk-dialog',
  'wk-follow-label',
  'wk-notice',
  'wk-preview',
  'wk-preview-body',
]

const note =
  'The ADE design system: tokens in src/index.css, components in src/components/{shadcn,ui}. See ade/web/AGENTS.md.'

export default defineConfig([
  {
    files: ['src/**/*.{ts,tsx}'],
    ignores: [
      'src/**/*.test.{ts,tsx}',
      'src/**/*.stories.tsx',
      'src/test/**',
      'src/**/__fixtures__/**',
    ],
    linterOptions: { reportUnusedDisableDirectives: 'off' },
    languageOptions: {
      parser: tsParser,
      parserOptions: { ecmaFeatures: { jsx: true } },
    },
    plugins: { shadcn },
    settings: {
      shadcn: {
        componentImports: ['^@/components/(shadcn|ui)(/|$)'],
        note,
      },
    },
    rules: {
      /* white/black are Tailwind built-ins; they still are not ours. */
      'shadcn/no-raw-colors': [
        'error',
        { allow: [], deny: ['*-white', '*-black'] },
      ],
      'shadcn/no-arbitrary-values': ['error', { allow: ['layout'] }],
      'shadcn/no-unknown-classes': [
        'error',
        { allow: [...ownClasses, ...hookClasses] },
      ],
      'shadcn/no-restyle': ['error', { allow: ['layout'] }],
      'shadcn/no-inline-styles': 'error',
      'shadcn/require-static-classes': 'error',
    },
  },
  {
    /* The design system itself: components own their appearance, so
       structural arbitrary values are fine. Colours (no-raw-colors skips
       arbitrary values, so `bg-[#…]` is caught only here) and inline styles
       stay checked here too. */
    files: ['src/components/shadcn/**', 'src/components/ui/**'],
    rules: {
      'shadcn/no-restyle': 'off',
      'shadcn/no-arbitrary-values': [
        'error',
        {
          allow: [
            'layout',
            'typography',
            'spacing',
            'shape',
            'effects',
            'motion',
          ],
        },
      ],
      'shadcn/require-static-classes': 'off',
    },
  },
])
