import { addons } from 'storybook/manager-api'
import { create } from 'storybook/theming'
import brandImageLight from '../src/icons/iii-ink.svg'
import brandImageDark from '../src/icons/iii-white.svg'

/** iii Schematic light ramp — matches index.css default tokens. */
export const lightTheme = create({
  base: 'light',
  brandTitle: 'iii console',
  brandImage: brandImageLight,
  brandTarget: '_self',
  appBg: '#f2f0ed',
  appContentBg: '#f2f0ed',
  appBorderColor: '#d8d5d0',
  barBg: '#e9e6e2',
  barTextColor: '#0a0a0a',
  barSelectedColor: '#b8420f',
  colorPrimary: '#b8420f',
  colorSecondary: '#6b6865',
  textColor: '#0a0a0a',
  inputBg: '#f2f0ed',
  inputBorder: '#d8d5d0',
  inputTextColor: '#0a0a0a',
  fontBase: '"Chivo", ui-monospace, monospace',
})

/** iii Schematic dark ramp — matches index.css `[data-theme="dark"]`. */
export const darkTheme = create({
  base: 'dark',
  brandTitle: 'iii console',
  brandImage: brandImageDark,
  brandTarget: '_self',
  appBg: '#111110',
  appContentBg: '#111110',
  appBorderColor: '#2a2926',
  barBg: '#1a1916',
  barTextColor: '#f2f0ed',
  barSelectedColor: '#3ea8ff',
  colorPrimary: '#3ea8ff',
  colorSecondary: '#a8a49e',
  textColor: '#f2f0ed',
  inputBg: '#111110',
  inputBorder: '#2a2926',
  inputTextColor: '#f2f0ed',
  fontBase: '"Chivo Mono", ui-monospace, monospace',
})

addons.setConfig({ theme: darkTheme })
