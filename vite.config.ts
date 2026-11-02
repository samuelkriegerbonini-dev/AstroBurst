import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

export default defineConfig({
  plugins: [react(), tailwindcss()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    allowedHosts: true,
    watch: {
      ignored: ["**/src-tauri/**", "**/docs/**", "**/entregas/**", "**/__tests__/**", "**/*.test.*", "**/ROADMAP.md"],
    },
  },
});
