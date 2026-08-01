import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";

// Vite config tailored for Tauri:
// - server.port 1420 must match tauri.conf.json devUrl
// - clearScreen false so Rust build output stays visible
// - envPrefix includes TAURI_ so @tauri-apps/api can read TAURI_* at runtime
// - server.strictPort: fail fast if 1420 is taken (Tauri dev won't find it)
export default defineConfig({
  plugins: [vue()],
  clearScreen: false,
  server: {
    port: 51820,
    strictPort: true,
    host: "127.0.0.1",
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
  envPrefix: ["VITE_", "TAURI_"],
  build: {
    target: "es2022",
    minify: "esbuild",
    sourcemap: true,
  },
  test: {
    environment: "jsdom",
    globals: false,
    include: ["tests/unit/**/*.spec.ts"],
  },
});