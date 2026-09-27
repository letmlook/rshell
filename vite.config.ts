import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";

// Vite config tailored for Tauri:
// - server.port 51820 must match tauri.conf.json devUrl
// - clearScreen false so Rust build output stays visible
// - envPrefix includes TAURI_ so @tauri-apps/api can read TAURI_* at runtime
// - server.strictPort: fail fast if 51820 is taken (Tauri dev won't find it)
// - sourcemap defaults to false in production; set RSHELL_SOURCEMAP=1 to opt in
// - manualChunks isolates vendor groups so no single JS chunk exceeds the
//   500 KiB threshold enforced by scripts/check-build-output.mjs
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
  envPrefix: ["VITE_", "TAURI_", "RSHELL_"],
  build: {
    target: "es2022",
    minify: "esbuild",
    sourcemap: process.env.RSHELL_SOURCEMAP === "1",
    chunkSizeWarningLimit: 500,
    rollupOptions: {
      output: {
        manualChunks: (id) => {
          if (!id.includes("node_modules")) return undefined;
          if (id.includes("dockview")) return "vendor-dockview";
          if (
            id.includes("element-plus") ||
            id.includes("@element-plus/icons-vue")
          ) {
            return groupElementPlus(id);
          }
          if (id.includes("@xterm") || id.includes("xterm")) return "vendor-xterm";
          if (id.includes("pinia")) return "vendor-pinia";
          if (id.includes("vue") || id.includes("@vue")) return "vendor-vue";
          return "vendor";
        },
      },
    },
  },
  test: {
    environment: "jsdom",
    globals: false,
    include: ["tests/unit/**/*.spec.ts"],
  },
});

// Split Element Plus by stable sub-area so its components are not one giant
// block. The components directory itself often exceeds the 500 KiB threshold
// when registered globally, so individual Vue components selectively import
// only the pieces they actually render.
function groupElementPlus(id) {
  if (id.includes("/components/")) return "vendor-element-plus-components";
  if (id.includes("/locale/")) return "vendor-element-plus-locale";
  if (id.includes("/directives/")) return "vendor-element-plus-directives";
  return "vendor-element-plus-core";
}