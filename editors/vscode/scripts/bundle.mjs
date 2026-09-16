// Put the language server beside the extension's own files, and write
// the licences that have to travel with it.
//
// `make vscode-package` and the release workflow both call this.
//
//   node scripts/bundle.mjs <path to sysml_wasm.wasm>
//
// One file for every machine and for none: no platform matrix here, and
// no binary to mark executable. The standard library is inside the
// module, which is why the licences below still have to travel.

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const extension = path.resolve(here, "..");
const root = path.resolve(extension, "..", "..");

const [module] = process.argv.slice(2);
if (!module) {
  console.error("usage: node scripts/bundle.mjs <path to sysml_wasm.wasm>");
  process.exit(2);
}
if (!fs.existsSync(module)) {
  console.error(
    `no server at ${module}; \`cargo build --release -p sysmlv2-wasm ` +
      "--target wasm32-unknown-unknown` writes one"
  );
  process.exit(1);
}

// where `src/extension.ts` looks for it
const server = path.join(extension, "server");
fs.rmSync(server, { recursive: true, force: true });
fs.mkdirSync(server, { recursive: true });
fs.copyFileSync(module, path.join(server, "sysml-lsp.wasm"));

// The library used to ship beside the server as files as well, for a
// server that could be pointed at a directory. This one has no
// filesystem to be pointed at, so they are gone from the package.
fs.rmSync(path.join(extension, "library"), { recursive: true, force: true });

// Carrying that library makes this package a distributor of EPL-2.0
// content, and EPL-2.0 asks a distributor to pass the licence and the
// notice along -- so this is not a tidiness question. The extension's
// own two licences travel in the same file, which is also what stops
// `vsce package` asking whether to go on without one.
const read = (...at) => fs.readFileSync(path.join(root, ...at), "utf8");
const said = [
  "The SysML v2 extension is part of sysml-v2-rs, which is licensed",
  "under either of the Apache License 2.0 or the MIT license, at your",
  "option.",
  "",
  "The language server it ships, `server/sysml-lsp.wasm`, carries inside",
  "it the KerML and SysML v2 standard model libraries, taken unchanged",
  "from the OMG SysML v2 Release",
  "(https://github.com/Systems-Modeling/SysML-v2-Release) and licensed",
  "under the Eclipse Public License 2.0. None of that library was written",
  "here and none of it was changed.",
  "",
  "The text of all three follows, and then that library's own notice.",
  "",
  "=== MIT ===",
  "",
  read("LICENSE-MIT"),
  "",
  "=== Apache License 2.0 ===",
  "",
  read("LICENSE-APACHE"),
  "",
  "=== Eclipse Public License 2.0 (for the library inside the server) ===",
  "",
  read("crates", "sysml-stdlib", "LICENSE"),
  "",
  "=== NOTICE (for the library inside the server) ===",
  "",
  read("crates", "sysml-stdlib", "NOTICE"),
].join("\n");
fs.writeFileSync(path.join(extension, "LICENSE.txt"), said);

const size = (fs.statSync(module).size / (1024 * 1024)).toFixed(1);
console.log(`bundled sysml-lsp.wasm (${size} MiB)`);
