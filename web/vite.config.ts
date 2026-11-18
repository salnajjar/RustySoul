import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

// One self-contained garden icon serves the UI, favicons and installed app.
const icons = ["rustysoul.svg"];
const iconFile = (name: string) =>
  new URL(`../images/${name}`, import.meta.url);

export default defineConfig({
  plugins: [
    react(),
    {
      name: "rustysoul-icons",
      buildStart() {
        for (const name of icons)
          this.addWatchFile(fileURLToPath(iconFile(name)));
      },
      configureServer(server) {
        server.middlewares.use((req, res, next) => {
          const pathname = req.url?.split("?")[0] || "";
          const name = pathname.startsWith("/assets/icons/")
            ? pathname.slice(14)
            : "";
          if (!icons.includes(name)) return next();
          res.setHeader("Content-Type", "image/svg+xml");
          res.end(readFileSync(iconFile(name)));
        });
      },
      generateBundle() {
        for (const name of icons)
          this.emitFile({
            type: "asset",
            fileName: `assets/icons/${name}`,
            source: readFileSync(iconFile(name)),
          });
      },
    },
  ],
  server: {
    proxy: { "/api": { target: "http://127.0.0.1:8484", changeOrigin: false } },
  },
  test: {
    environment: "jsdom",
    setupFiles: "./src/test-setup.ts",
    include: ["src/**/*.test.ts", "src/**/*.test.tsx"],
  },
});
