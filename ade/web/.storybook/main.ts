import { fileURLToPath, URL } from 'node:url'
import type { StorybookConfig } from '@storybook/react-vite'
import { mergeConfig } from 'vite'

const config: StorybookConfig = {
  stories: ['../src/**/*.mdx', '../src/**/*.stories.@(ts|tsx)'],
  staticDirs: ['../.storybook/public', { from: '../src/icons', to: '/icons' }],
  addons: ['@storybook/addon-docs', '@storybook/addon-a11y'],
  framework: {
    name: '@storybook/react-vite',
    options: {},
  },
  // Mirror the `@` -> `src` alias from vite.config.ts. The Tailwind v4 plugin
  // and the React plugin are inherited from the project's vite config, so the
  // iii Schematic styles (index.css) resolve exactly as they do in the app.
  viteFinal(viteConfig) {
    return mergeConfig(viteConfig, {
      resolve: {
        alias: {
          '@': fileURLToPath(new URL('../src', import.meta.url)),
        },
      },
    })
  },
}

export default config
