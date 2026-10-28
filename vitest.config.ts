import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  test: {
    environment: "jsdom",
    // issue #248: 打开 CSS 处理 —— 这样测试可以用 `?raw` 读到样式源码，
    // 对「行高单一来源」「token 度量中性」这类只在真实渲染层成立的
    // 不变量立护栏（jsdom 不排版，DOM 断言抓不到度量类回归）。
    // 副作用是 CSS Module 的类名变成哈希值，测试应按 `[class*=...]` 匹配。
    css: true,
    globals: true,
    setupFiles: ["./src/test/setup.ts"],
    coverage: {
      reporter: ["text", "json", "html"],
      exclude: ["node_modules/", "src/test/"],
    },
  },
});
