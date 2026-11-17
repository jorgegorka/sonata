/// <reference types="vitest" />
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { resolve } from "node:path";

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  build: {
    rollupOptions: {
      input: { main: resolve(__dirname, "index.html"), pickbox: resolve(__dirname, "pickbox.html") },
      output: { manualChunks: { xterm: ["@xterm/xterm"], flow: ["@xyflow/react", "@dagrejs/dagre"] } },
    },
  },
  test: { environment: "node", css: { include: [/styles\.css/] } },
});
