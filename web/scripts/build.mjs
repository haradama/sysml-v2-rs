// Build the playground into `dist/`, which is the whole site.
//
//   dist/index.html            the page
//   dist/style.css             and what it is painted in
//   dist/app.js                the front end
//   dist/server.worker.js      the worker the language server runs in
//   dist/sysml-lsp.wasm        the language server
//   dist/sysml.tmLanguage.json what colours the editor
//   dist/onig.wasm             the engine that reads that grammar
//   dist/LICENSE.txt           what has to travel with it
//
// Four static files and a module. There is no server here in any sense:
// GitHub Pages hands these over and the tab does the rest.
//
//   node scripts/build.mjs [--wasm <path>] [--watch] [--serve]

import esbuild from "esbuild";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const web = path.resolve(here, "..");
const root = path.resolve(web, "..");

const flag = (name) => process.argv.includes(`--${name}`);
const option = (name) => {
  const at = process.argv.indexOf(`--${name}`);
  return at === -1 ? undefined : process.argv[at + 1];
};

const watching = flag("watch") || flag("serve");
const dist = path.join(web, "dist");
const module =
  option("wasm") ??
  path.join(root, "target", "wasm32-unknown-unknown", "wasm", "sysml_wasm.wasm");

if (!fs.existsSync(module)) {
  console.error(
    `no language server at ${module}\n` +
      "  cargo build -p sysmlv2-wasm --target wasm32-unknown-unknown --profile wasm"
  );
  process.exit(1);
}

// emptied first, so that what is published is what this script wrote
fs.rmSync(dist, { recursive: true, force: true });
fs.mkdirSync(dist, { recursive: true });

const shared = {
  bundle: true,
  sourcemap: true,
  minify: !watching,
  target: "es2021",
  logLevel: "info",
  absWorkingDir: web,
};

const builds = [
  {
    ...shared,
    entryPoints: ["src/app.ts"],
    outfile: "dist/app.js",
    format: "esm",
    platform: "browser",
  },
  {
    ...shared,
    // The VSCode extension's worker, bundled from where it lives rather
    // than copied here. It is the whole of what the module's four
    // exports ask for -- write a message into the buffer, say how long
    // it is, read the answers back out -- and two copies of that would
    // be one copy that quietly stopped matching the .wasm. It imports
    // nothing, so nothing of the extension comes with it.
    entryPoints: [path.join(root, "editors", "vscode", "src", "server.worker.ts")],
    outfile: "dist/server.worker.js",
    // a worker is loaded by URL, with no module loader around it
    format: "iife",
    platform: "browser",
  },
];

/// The page and everything beside it, copied as they are.
function statics() {
  fs.cpSync(path.join(web, "static"), dist, { recursive: true });
  fs.copyFileSync(module, path.join(dist, "sysml-lsp.wasm"));
  // The extension's grammar, taken from where it lives rather than
  // copied into this directory: it is the one description of what SysML
  // v2 looks like that this repository keeps, and
  // `editors/vscode/test/grammar.test.mjs` holds it to the reserved
  // words in the lexer's own table.
  fs.copyFileSync(
    path.join(root, "editors", "vscode", "syntaxes", "sysml.tmLanguage.json"),
    path.join(dist, "sysml.tmLanguage.json")
  );
  // and the regular-expression engine VSCode reads that grammar with,
  // which `vscode-oniguruma` ships as a module of its own
  fs.copyFileSync(
    path.join(web, "node_modules", "vscode-oniguruma", "release", "onig.wasm"),
    path.join(dist, "onig.wasm")
  );
  // Pages runs Jekyll over what it is given unless told not to, and
  // Jekyll drops files whose names begin with an underscore.
  fs.writeFileSync(path.join(dist, ".nojekyll"), "");
  fs.writeFileSync(path.join(dist, "LICENSE.txt"), licences());
}

/// The licences the site distributes.
///
/// The standard library is inside the .wasm, so publishing this makes
/// the site a distributor of EPL-2.0 content, and EPL-2.0 asks a
/// distributor to pass the licence and the notice along. Same obligation
/// as the VSCode extension's `scripts/bundle.mjs`, same answer.
///
/// This one carries two more than the extension does, because it ships
/// what the extension only tests with: `vscode-textmate` is bundled into
/// `app.js` and `onig.wasm` is Oniguruma itself, under a licence that
/// asks a binary redistribution to reproduce its notice. They are read
/// out of `node_modules` rather than copied here, so what is published
/// is the licence of the version that was published with it.
function licences() {
  const read = (...at) => fs.readFileSync(path.join(root, ...at), "utf8");
  const npm = (...at) => fs.readFileSync(path.join(web, "node_modules", ...at), "utf8");
  return [
    "The SysML v2 playground is part of sysml-v2-rs, which is licensed",
    "under either of the Apache License 2.0 or the MIT license, at your",
    "option.",
    "",
    "The language server it loads, `sysml-lsp.wasm`, carries inside it the",
    "KerML and SysML v2 standard model libraries, taken unchanged from the",
    "OMG SysML v2 Release",
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
    "",
    "The editor is coloured by the same TextMate grammar the VSCode",
    "extension ships, read by `vscode-textmate` (bundled into `app.js`)",
    "and `onig.wasm`, which is Oniguruma. Their licences and notices",
    "follow.",
    "",
    "=== MIT (vscode-textmate) ===",
    "",
    npm("vscode-textmate", "LICENSE.md"),
    "",
    "=== MIT (vscode-oniguruma) ===",
    "",
    npm("vscode-oniguruma", "LICENSE.txt"),
    "",
    "=== NOTICES (for Oniguruma, inside onig.wasm) ===",
    "",
    npm("vscode-oniguruma", "NOTICES.txt"),
  ].join("\n");
}

statics();

if (watching) {
  const contexts = await Promise.all(builds.map((it) => esbuild.context(it)));
  await Promise.all(contexts.map((it) => it.watch()));
  // the page and the stylesheet are copied, not built, so esbuild's
  // watcher never sees them
  fs.watch(path.join(web, "static"), { recursive: true }, () => {
    fs.cpSync(path.join(web, "static"), dist, { recursive: true });
  });
  if (flag("serve")) {
    const { host, port } = await contexts[0].serve({ servedir: "dist", port: 8000 });
    console.log(`serving http://${host === "0.0.0.0" ? "localhost" : host}:${port}`);
  } else {
    console.log("watching");
  }
} else {
  await Promise.all(builds.map((it) => esbuild.build(it)));
  const mib = (at) => (fs.statSync(at).size / (1024 * 1024)).toFixed(1);
  console.log(
    `built dist/ (sysml-lsp.wasm ${mib(module)} MiB, ` +
      `onig.wasm ${mib(path.join(dist, "onig.wasm"))} MiB)`
  );
}
