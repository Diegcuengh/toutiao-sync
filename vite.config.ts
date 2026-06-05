import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";

export default defineConfig({
  plugins: [vue()],
  clearScreen: false,
  server: {
    port: 14321,
    strictPort: true,
  },
  envPrefix: ["VITE_", "TAURI_"],
});
