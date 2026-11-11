import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// https://vite.dev/config/
export default defineConfig({
  plugins: [react()],
  // No ocultar los errores de Rust durante `tauri dev`.
  clearScreen: false,
  server: {
    // Tauri espera un puerto fijo (tauri.conf.json → build.devUrl).
    port: 1420,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**", "**/crates/**", "**/target/**"] },
  },
});
