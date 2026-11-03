import { realpathSync } from 'node:fs'
import { createRequire } from 'node:module'
import { fileURLToPath } from 'node:url'
const require = createRequire(new URL('../../../ade/web/package.json', import.meta.url))
const { defineConfig } = await import(require.resolve('vite'))
const { default: react } = await import(require.resolve('@vitejs/plugin-react'))
const { default: tailwindcss } = await import(require.resolve('@tailwindcss/vite'))
export default defineConfig({
  root: fileURLToPath(new URL('.', import.meta.url)),
  plugins: [react(), tailwindcss()],
  resolve: { dedupe: ['react', 'react-dom'], alias: [
    { find: /^@iii-dev\/console-ui$/, replacement: fileURLToPath(new URL('./console-ui.ts', import.meta.url)) },
    { find: '@', replacement: fileURLToPath(new URL('../../../ade/web/src', import.meta.url)) },
  ] },
  server: { host: '127.0.0.1', port: 4197, strictPort: true, hmr: false, ws: false, fs: { allow: [fileURLToPath(new URL('../../../', import.meta.url)), realpathSync(fileURLToPath(new URL('../../../node_modules', import.meta.url))), realpathSync(fileURLToPath(new URL('../../../ade/web/node_modules', import.meta.url)))] } },
})
