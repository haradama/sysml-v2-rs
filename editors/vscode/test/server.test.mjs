// The language server the extension ships, driven the way the worker
// drives it.
//
// `out/server.worker.js` is loaded into a context with the globals a
// worker has, handed the module, and then spoken to in the Language
// Server Protocol -- so what is checked is the whole crossing, against
// the `.wasm` that will ship rather than a copy of the arrangement
// written out again here.

import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import test from "node:test";
import vm from "node:vm";

const WASM = new URL("../server/sysml-lsp.wasm", import.meta.url);
const WORKER = new URL("../out/server.worker.js", import.meta.url);

// `make vscode-package` builds the module; a checkout that has only run
// `npm install` has not got one, and a test that cannot run says so.
const built = existsSync(fileURLToPath(WASM));
const unbuilt = {
  skip: built
    ? false
    : "no server/sysml-lsp.wasm -- run `make vscode-package` (or `cargo build " +
      "-p sysmlv2-wasm --target wasm32-unknown-unknown --profile wasm`)",
};

/// The worker, running the module, with the handles a test drives it by.
function serving() {
  const posted = [];
  const sandbox = {
    postMessage: (message) => posted.push(message),
    TextEncoder,
    TextDecoder,
    WebAssembly,
    // A worker is one realm, and this is two: without these the
    // answers come back as arrays and objects built by the context's
    // own constructors, which are not the ones this file compares
    // against.
    JSON,
    Uint8Array,
    console,
    onmessage: null,
  };
  vm.createContext(sandbox);
  vm.runInContext(readFileSync(fileURLToPath(WORKER), "utf8"), sandbox);
  sandbox.onmessage({ data: { wasm: readFileSync(fileURLToPath(WASM)) } });

  let id = 0;
  return {
    posted,
    /// Say something to the server, as the client does.
    say: (message) => sandbox.onmessage({ data: { jsonrpc: "2.0", ...message } }),
    /// Ask it something, and hand back the id to wait for the answer on.
    ask(method, params) {
      id += 1;
      this.say({ id, method, params });
      return id;
    },
    /// Wait for a message that answers `wanted`. The module is compiled
    /// and the standard library is read while the first of these waits.
    async wait(wanted, why) {
      for (let tick = 0; tick < 600; tick += 1) {
        const found = posted.find(wanted);
        if (found) {
          return found;
        }
        await new Promise((resume) => setTimeout(resume, 25));
      }
      assert.fail(`${why}; the server said ${JSON.stringify(posted, null, 2)}`);
    },
  };
}

/// The handshake, with the project's files handed over in it.
function initialize(server, files) {
  return server.ask("initialize", {
    capabilities: {},
    workspaceFolders: [{ uri: "file:///model", name: "model" }],
    initializationOptions: { files },
  });
}

const diagnostics = (uri) => (message) =>
  message.method === "textDocument/publishDiagnostics" &&
  message.params.uri === uri;

function open(server, uri, text) {
  server.say({
    method: "textDocument/didOpen",
    params: {
      textDocument: { uri, languageId: "sysml", version: 1, text },
    },
  });
}

test("the module answers the handshake with what it can do", unbuilt, async () => {
  const server = serving();
  const id = initialize(server, []);
  const answer = await server.wait(
    (message) => message.id === id,
    "no answer to `initialize`"
  );
  assert.equal(answer.jsonrpc, "2.0");
  assert.equal(answer.result.capabilities.renameProvider, true);
  assert.equal(answer.result.capabilities.definitionProvider, true);
});

test("a name from the standard library resolves, with no library on disk", unbuilt, async () => {
  const server = serving();
  initialize(server, []);
  server.say({ method: "initialized", params: {} });

  const uri = "file:///model/car.sysml";
  // `ISQ::MassValue` is the library's, and the library is inside the
  // module: there is nowhere else it could be read from here
  open(server, uri, "part def Car {\n    attribute mass : ISQ::MassValue;\n}\n");
  const said = await server.wait(diagnostics(uri), "nothing was published");
  assert.deepEqual(
    said.params.diagnostics.map((it) => it.message),
    [],
    "the model is well formed"
  );
});

test("a name that resolves to nothing is reported where it is written", unbuilt, async () => {
  const server = serving();
  initialize(server, []);
  server.say({ method: "initialized", params: {} });

  const uri = "file:///model/typo.sysml";
  open(server, uri, "part def Car {\n    part w : Wheeel;\n}\n");
  const said = await server.wait(diagnostics(uri), "nothing was published");
  const [first] = said.params.diagnostics;
  assert.match(first.message, /Wheeel/);
  assert.equal(first.range.start.line, 1);
});

test("a document resolves against the files the client handed over", unbuilt, async () => {
  const server = serving();
  // the other half of the model, which no server here could have found
  // for itself
  initialize(server, [
    { uri: "file:///model/parts.sysml", text: "part def Wheel;\n" },
  ]);
  server.say({ method: "initialized", params: {} });

  const uri = "file:///model/car.sysml";
  open(server, uri, "part def Car {\n    part w : Wheel[4];\n}\n");
  const said = await server.wait(diagnostics(uri), "nothing was published");
  assert.deepEqual(
    said.params.diagnostics.map((it) => it.message),
    [],
    "`Wheel` is declared next door"
  );
});

test("a file handed over later is read", unbuilt, async () => {
  const server = serving();
  initialize(server, []);
  server.say({ method: "initialized", params: {} });

  const uri = "file:///model/car.sysml";
  open(server, uri, "part def Car {\n    part w : Wheel[4];\n}\n");
  const first = await server.wait(diagnostics(uri), "nothing was published");
  assert.equal(first.params.diagnostics.length, 1, "`Wheel` is nowhere yet");

  // a file written outside the editor, which the watcher picks up
  server.posted.length = 0;
  server.say({
    method: "sysml/files",
    params: {
      files: [{ uri: "file:///model/parts.sysml", text: "part def Wheel;\n" }],
    },
  });
  const then = await server.wait(diagnostics(uri), "nothing was published again");
  assert.deepEqual(
    then.params.diagnostics.map((it) => it.message),
    [],
    "and now it is declared"
  );
});

test("the preview is drawn by the same module", unbuilt, async () => {
  const server = serving();
  initialize(server, []);
  server.say({ method: "initialized", params: {} });

  const uri = "file:///model/car.sysml";
  open(server, uri, "part def PowerSource;\npart def Engine :> PowerSource;\n");
  await server.wait(diagnostics(uri), "nothing was published");

  const id = server.ask("sysml/diagram", { uri, view: "definitions" });
  const answer = await server.wait(
    (message) => message.id === id,
    "no drawing came back"
  );
  assert.match(answer.result.svg, /^<svg xmlns=/);
  assert.match(answer.result.svg, /Engine/);
});
