import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

export default defineConfig({
  base: './',
  server: { fs: { allow: ['..'] } },
  plugins: [react()],
})
