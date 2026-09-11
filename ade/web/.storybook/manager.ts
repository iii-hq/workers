import '@fontsource/geist/400.css'
import '@fontsource/geist/500.css'
import '@fontsource/geist/600.css'
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
  fontBase: '"Geist", ui-sans-serif, system-ui, sans-serif',
})

/** iii Schematic dark ramp — matches index.css `[data-theme="dark"]`. */
export const darkTheme = create({
  base: 'dark',
  brandTitle: 'iii console',
  brandImage: brandImageDark,
  brandTarget: '_self',
  appBg: '#0a0a0a',
  appContentBg: '#0a0a0a',
  appBorderColor: '#1f1f1f',
  barBg: '#111111',
  barTextColor: '#ededed',
  barSelectedColor: '#28a8f7',
  colorPrimary: '#28a8f7',
  colorSecondary: '#a6a6a6',
  textColor: '#ededed',
  inputBg: '#0a0a0a',
  inputBorder: '#1f1f1f',
  inputTextColor: '#ededed',
  fontBase: '"Geist", ui-sans-serif, system-ui, sans-serif',
})

addons.setConfig({ theme: darkTheme })
