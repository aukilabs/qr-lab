/// <reference types="vitest/config" />
import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

// https://vite.dev/config/ + https://vitest.dev/config/
export default defineConfig({
  plugins: [react()],
  test: {
    environment: "node",
    exclude: process.env.QR_LAB_WASM_TESTS ? [] : ["**/*.wasm.test.ts"],
    include: ["src/**/*.test.{ts,tsx}", "scripts/**/*.test.mjs"],
  },
});
