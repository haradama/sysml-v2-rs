// The language server, in a worker.
//
// The whole of what the WebAssembly module asks for: a message is
// written into its one buffer, it is told how long that message is, and
// the answers come back out of the same buffer.
//
// Everything slow happens on this side of `postMessage` -- half a second
// to read the standard library when the client says `initialize`, and
// single-figure milliseconds a keystroke after that.
//
// The playground in `web/` bundles this file too: it is the whole of
// what the module's four exports ask for, and what it may use is what a
// worker has, not what VSCode has. Nothing VSCode-shaped belongs here.

/// What the module exports. See `crates/sysml-wasm`.
interface Server {
  memory: WebAssembly.Memory;
  /// Make room for a message of `length` bytes, and say where to write it.
  take(length: number): number;
  /// Answer the message now in the buffer; the return is how long the
  /// answers are.
  handle(length: number): number;
  /// Where those answers begin.
  answers(): number;
  /// Whether the session is finished with.
  finished(): number;
  /// What the last panic said, left in the buffer.
  trap(): number;
}

const encoder = new TextEncoder();
const decoder = new TextDecoder();

let server: Server | undefined;
/// Messages that arrived while the module was still being compiled: the
/// client starts talking as soon as it has a worker, and the first
/// thing it says is the handshake.
const waiting: unknown[] = [];

/// Read `length` bytes from `at` in the module's memory. The view is
/// made afresh every time, since growing that memory -- which reading a
/// model does -- replaces the buffer every view was made against.
function decode(server: Server, at: number, length: number): string {
  return decoder.decode(new Uint8Array(server.memory.buffer, at, length));
}

/// Give the server one message and post back whatever answers it.
function deliver(server: Server, message: unknown): void {
  const bytes = encoder.encode(JSON.stringify(message));
  const at = server.take(bytes.length);
  new Uint8Array(server.memory.buffer, at, bytes.length).set(bytes);
  let length: number;
  try {
    length = server.handle(bytes.length);
  } catch (err) {
    // a panic reaches here as a trap with nothing in it, and the module
    // is the only place what it said still exists
    const said = server.trap();
    const why = said > 0 ? decode(server, server.answers(), said) : `${err}`;
    log(`the language server stopped: ${why}`);
    return;
  }
  const answers = JSON.parse(decode(server, server.answers(), length));
  for (const answer of answers) {
    postMessage(answer);
  }
}

/// Say something in the client's log.
function log(message: string): void {
  postMessage({
    jsonrpc: "2.0",
    method: "window/logMessage",
    params: { type: 1, message: `sysml-lsp: ${message}` },
  });
}

async function boot(bytes: BufferSource): Promise<void> {
  const { instance } = await WebAssembly.instantiate(bytes, {});
  server = instance.exports as unknown as Server;
  for (const message of waiting.splice(0)) {
    deliver(server, message);
  }
}

onmessage = (event: MessageEvent) => {
  const said = event.data;
  // the extension hands the module over first, before the client has
  // said anything
  if (said && said.wasm) {
    void boot(said.wasm);
    return;
  }
  if (!server) {
    waiting.push(said);
    return;
  }
  deliver(server, said);
};
