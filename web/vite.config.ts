import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// In dev, proxy the API (and SSE) to the Rust server on :8080.
export default defineConfig({
  plugins: [react()],
  // Relative asset URLs (FEAT-178), so the same build works from any folder on any static host.
  // The daemon injects `<base href="/">` into the index it serves, so a deep link such as
  // /p/x/feature/Y still loads ./assets/… from the root.
  base: "./",
  server: {
    port: 5173,
    proxy: {
      "/api": {
        target: "http://localhost:8080",
        changeOrigin: true,
      },
    },
  },
});
