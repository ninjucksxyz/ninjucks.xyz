import * as esbuild from "esbuild";
import { polyfillNode } from "esbuild-plugin-polyfill-node";
await esbuild.build({
  entryPoints: ["entry.mjs"],
  bundle: true, format: "esm", minify: true, target: "es2020",
  outfile: "../frontend/vendor/injective.min.js",
  mainFields: ["main"],            // prefer CommonJS dist (ESM dist has broken cross-exports)
  conditions: ["require", "node"],
  define: { "process.env.NODE_ENV": '"production"', global: "globalThis" },
  plugins: [polyfillNode({ polyfills: { crypto: true, buffer: true, stream: true, process: true, events: true } })],
  legalComments: "none",
});
console.log("bundle built");
