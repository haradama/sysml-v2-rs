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

/// `WebAssembly` as the worker sees it, with the module's first instance
/// trapping on the first message that asks `trapOn` -- and on every call
/// after it, as a module left mid-call by a panic does.
///
/// The server has no panic left to set one off with on purpose, and
/// keeping one in it for a test's sake is keeping a bug; so the trap is
/// set off here, at the one boundary the worker can see, while every
/// answer still comes from the module that ships.
function trapping(trapOn) {
  let traps = trapOn ? 1 : 0;
  const wrap = (instance) => {
    const real = instance.exports;
    let at = 0;
    let dead = false;
    const trap = () => {
      dead = true;
      throw new WebAssembly.RuntimeError("unreachable executed");
    };
    const exports = {
      ...real,
      take(length) {
        if (dead) trap();
        at = real.take(length);
        return at;
      },
      handle(length) {
        if (dead) trap();
        const said = new TextDecoder().decode(new Uint8Array(real.memory.buffer, at, length));
        if (traps > 0 && JSON.parse(said).method === trapOn) {
          traps -= 1;
          trap();
        }
        return real.handle(length);
      },
      answers: () => (dead ? trap() : real.answers()),
    };
    return { exports };
  };
  return {
    ...WebAssembly,
    RuntimeError: WebAssembly.RuntimeError,
    compile: (bytes) => WebAssembly.compile(bytes),
    instantiate: async (module, imports) => wrap(await WebAssembly.instantiate(module, imports)),
    Instance: function Instance(module, imports) {
      return wrap(new WebAssembly.Instance(module, imports));
    },
  };
}

/// The worker, running the module, with the handles a test drives it by.
function serving({ trapOn } = {}) {
  const posted = [];
  const sandbox = {
    postMessage: (message) => posted.push(message),
    TextEncoder,
    TextDecoder,
    WebAssembly: trapOn ? trapping(trapOn) : WebAssembly,
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

/// The module is built with `panic = "abort"`, so a panic is a trap that
/// leaves the instance unusable. The question that trapped is answered
/// with an error rather than left waiting, and a new instance is told
/// what the old one knew -- the handshake, the handed-over files and
/// each open document as it reads after its edits -- so the next
/// question is answered as though nothing had happened.
///
/// The trap is set off by `trapping`, on the first `codeAction` asked.
test("a panic answers its question and the server goes on", unbuilt, async () => {
  const server = serving({ trapOn: "textDocument/codeAction" });
  initialize(server, [{ uri: "file:///model/parts.sysml", text: "part def Wheel;\n" }]);
  server.say({ method: "initialized", params: {} });

  const uri = "file:///model/car.sysml";
  open(server, uri, "part def Car {\n    part w : Wheeel;\n}\n");
  await server.wait(diagnostics(uri), "nothing was published");
  // an edit, so that what is replayed is the document as edited
  server.say({
    method: "textDocument/didChange",
    params: {
      textDocument: { uri, version: 2 },
      contentChanges: [
        { range: { start: { line: 1, character: 15 }, end: { line: 1, character: 16 } }, text: "" },
      ],
    },
  });

  server.posted.length = 0;
  const trapped = server.ask("textDocument/codeAction", {
    textDocument: { uri },
    range: { start: { line: 0, character: 0 }, end: { line: 1, character: 5 } },
    context: { diagnostics: [] },
  });
  const refused = await server.wait(
    (message) => message.id === trapped,
    "the question that trapped was never answered"
  );
  assert.equal(refused.error.code, -32603);
  assert.match(refused.error.message, /codeAction/);
  // the new instance republishes what it makes of the document: the
  // edit fixed the typo, against a file handed over in the handshake
  const republished = await server.wait(diagnostics(uri), "nothing was republished");
  assert.deepEqual(republished.params.diagnostics, [], "the edit and the handed file both reached it");

  const id = server.ask("textDocument/hover", {
    textDocument: { uri },
    position: { line: 1, character: 15 },
  });
  const hovered = await server.wait((message) => message.id === id, "no answer after the restart");
  assert.equal(hovered.error, undefined, JSON.stringify(hovered));
  assert.match(JSON.stringify(hovered.result), /Wheel/);
});
