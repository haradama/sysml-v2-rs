// Bundle the extension.
//
//   out/extension.js     what VSCode loads, in a worker extension host
//   out/server.worker.js the worker the language server runs in
//   out/page.js          the preview's page, which is also what
//                        `test/page.test.mjs` drives under a DOM
//
// Bundled rather than compiled file by file because none of the hosts
// this runs in resolves `node_modules` at run time -- which is what the
// list of packages in `.vscodeignore` used to be for.
//
//   node scripts/build.mjs [--watch]

import esbuild from "esbuild";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const extension = path.resolve(here, "..");
const watching = process.argv.includes("--watch");

// emptied first, so that what ships is what this script wrote: a file
// left by an earlier build was packaged along with the rest and nobody
// could tell by looking
fs.rmSync(path.join(extension, "out"), { recursive: true, force: true });

const shared = {
  bundle: true,
  sourcemap: true,
  minify: false,
  target: "es2021",
  logLevel: "info",
  absWorkingDir: extension,
};

const builds = [
  {
    ...shared,
    entryPoints: ["src/extension.ts"],
    outfile: "out/extension.js",
    // VSCode's own module is provided by the host, never bundled
    external: ["vscode"],
    format: "cjs",
    platform: "browser",
  },
  {
    ...shared,
    entryPoints: ["src/server.worker.ts"],
    outfile: "out/server.worker.js",
    // a worker is loaded by URL and has no module loader around it
    format: "iife",
    platform: "browser",
  },
  {
    ...shared,
    entryPoints: ["src/page.ts"],
    outfile: "out/page.js",
    // required by the test, which is Node
    format: "cjs",
    platform: "neutral",
  },
];

if (watching) {
  const contexts = await Promise.all(builds.map((it) => esbuild.context(it)));
  await Promise.all(contexts.map((it) => it.watch()));
  console.log("watching");
} else {
  await Promise.all(builds.map((it) => esbuild.build(it)));
}
