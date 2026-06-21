import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
// @ts-expect-error type error without @types/node package
import process from "node:process";
// @ts-expect-error type error without @types/node package
import fs from "node:fs";
// @ts-expect-error type error without @types/node package
import path from "node:path";

const host = process.env.TAURI_DEV_HOST;

// Read version from package.json
const packageJson = JSON.parse(
  fs.readFileSync(path.resolve(process.cwd(), "package.json"), "utf-8")
);

// https://vite.dev/config/
export default defineConfig(() => ({
  plugins: [react()],
  base: "./",
  define: {
    __APP_VERSION__: JSON.stringify(packageJson.version),
  },

  // Perf fix (#37): Code splitting to reduce initial bundle size
  build: {
    rollupOptions: {
      output: {
        manualChunks: {
          react: ["react", "react-dom", "react-router-dom"],
          tauri: ["@tauri-apps/api", "@tauri-apps/plugin-dialog"],
        },
      },
    },
  },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
}));
