import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// In dev, proxy the API (and SSE) to the Rust server on :8080.
export default defineConfig({
  plugins: [react()],
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
