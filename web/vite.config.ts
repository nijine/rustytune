import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// In dev (`npm run dev`) Vite serves the frontend with HMR and proxies API
// calls to a locally running `rustytune` server. Production builds land in
// dist/, which the server embeds at compile time.
export default defineConfig({
  plugins: [react()],
  server: {
    proxy: {
      "/api": { target: "http://127.0.0.1:8642", ws: true },
    },
  },
});
