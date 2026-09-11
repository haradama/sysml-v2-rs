// Put the language server and the standard library beside the
// extension's own files, and write the licences that have to travel with
// them.
//
// `make vscode-package` and the release workflow both call this. The
// workflow packages for Windows as well, where the Makefile's shell is
// not there to do it.
//
//   node scripts/bundle.mjs <path to the sysml-lsp binary>

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const extension = path.resolve(here, "..");
const root = path.resolve(extension, "..", "..");

const [binary] = process.argv.slice(2);
if (!binary) {
  console.error("usage: node scripts/bundle.mjs <path to sysml-lsp>");
  process.exit(2);
}
if (!fs.existsSync(binary)) {
  console.error(`no server binary at ${binary}; \`cargo build --release -p sysmlv2-lsp\` writes one`);
  process.exit(1);
}

// the extension looks for these two beside its own files before it falls
// back to a setting or to PATH
const server = path.join(extension, "server");
fs.rmSync(server, { recursive: true, force: true });
fs.mkdirSync(server, { recursive: true });
const named = path.extname(binary) === ".exe" ? "sysml-lsp.exe" : "sysml-lsp";
fs.copyFileSync(binary, path.join(server, named));
fs.chmodSync(path.join(server, named), 0o755);

// the copy that ships is `sysml-stdlib`'s, not the submodule's: that
// crate carries the standard library and the server has it built in, so
// it is what the extension would be pointed at anyway
const library = path.join(extension, "library", "sysml.library");
fs.rmSync(path.join(extension, "library"), { recursive: true, force: true });
fs.cpSync(path.join(root, "crates", "sysml-stdlib", "library"), library, { recursive: true });

// Bundling `library/` makes this package a distributor of EPL-2.0
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
  "It also carries, under `library/`, the KerML and SysML v2 standard",
  "model libraries, taken unchanged from the OMG SysML v2 Release",
  "(https://github.com/Systems-Modeling/SysML-v2-Release) and licensed",
  "under the Eclipse Public License 2.0. Nothing in `library/` was",
  "written here and nothing in it was changed.",
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
  "=== Eclipse Public License 2.0 (for library/) ===",
  "",
  read("crates", "sysml-stdlib", "LICENSE"),
  "",
  "=== NOTICE (for library/) ===",
  "",
  read("crates", "sysml-stdlib", "NOTICE"),
].join("\n");
fs.writeFileSync(path.join(extension, "LICENSE.txt"), said);

const files = fs.readdirSync(library, { recursive: true }).length;
console.log(`bundled ${named} and ${files} library files`);
