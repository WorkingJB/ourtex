import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Dev server proxies `/v1/*` and `/healthz` to the local orchext-server
// so the browser can hit `fetch("/v1/auth/login")` without CORS setup.
// ORCHEXT_SERVER_URL overrides the default if the server isn't on :8080.
const serverUrl = process.env.ORCHEXT_SERVER_URL ?? "http://localhost:8080";

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  // Enable sourcemaps in production builds so the test deploy's
  // stack traces are decodable from the browser console while we
  // chase the double-click bold bug. Bundle size impact is small
  // and the maps only download when devtools is open.
  build: {
    sourcemap: true,
  },
  server: {
    port: 1430,
    strictPort: true,
    proxy: {
      "/v1": { target: serverUrl, changeOrigin: true },
      "/healthz": { target: serverUrl, changeOrigin: true },
    },
  },
});
