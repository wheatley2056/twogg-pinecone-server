import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Wird vom Server unter /admin ausgeliefert; im Entwicklungsmodus leitet Vite /api an den
// lokal laufenden Server (cargo run -p pinecone-server) weiter.
export default defineConfig({
  base: "/admin/",
  plugins: [react()],
  server: {
    port: 5174,
    proxy: { "/api": "http://localhost:8080", "/media": "http://localhost:8080" },
  },
  build: { outDir: "dist", emptyOutDir: true, chunkSizeWarningLimit: 2500 },
});
