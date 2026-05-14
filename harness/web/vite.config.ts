import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  server: {
    port: 5173,
    proxy: {
      "/harness": {
        target: "http://127.0.0.1:3111",
        changeOrigin: false,
      },
      "/iii/ws": {
        target: "ws://127.0.0.1:49134",
        ws: true,
        changeOrigin: false,
        rewrite: (path: string) => path.replace(/^\/iii\/ws/, ""),
      },
    },
  },
});
