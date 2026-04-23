import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Dev server proxies `/v1/*` and `/healthz` to the local ourtex-server
// so the browser can hit `fetch("/v1/auth/login")` without CORS setup.
// OURTEX_SERVER_URL overrides the default if the server isn't on :8080.
const serverUrl = process.env.OURTEX_SERVER_URL ?? "http://localhost:8080";

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1430,
    strictPort: true,
    proxy: {
      "/v1": { target: serverUrl, changeOrigin: true },
      "/healthz": { target: serverUrl, changeOrigin: true },
    },
  },
});
