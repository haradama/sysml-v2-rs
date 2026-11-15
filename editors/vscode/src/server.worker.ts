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

/// The compiled module, kept so that a server that trapped can be
/// started again without compiling six megabytes a second time.
let compiled: WebAssembly.Module | undefined;
let server: Server | undefined;
/// Messages that arrived while the module was still being compiled: the
/// client starts talking as soon as it has a worker, and the first
/// thing it says is the handshake.
const waiting: unknown[] = [];
/// A server that trapped while being brought back, which would only
/// trap again: from then on a question is answered with an error, so
/// that nobody waits for an answer that is not coming.
let givenUp: string | undefined;

/// A message of the Language Server Protocol, as far as this file reads one.
interface Said {
  id?: number | string | null;
  method?: string;
  params?: any;
}

/// What a new server has to be told to be where the one that trapped
/// was. The module is built with `panic = "abort"` -- nothing unwinds
/// in WebAssembly without an exception-handling toolchain -- so a panic
/// is a trap, and a module that trapped is left mid-call, every later
/// call to it trapping too. The only way on is a new instance, and this
/// is what it is told before anything else: the handshake, the files
/// handed over since, and each open document as it reads now.
const session = {
  initialize: undefined as Said | undefined,
  initialized: false,
  /// `sysml/files`, the last text each file was handed over with.
  files: new Map<string, string | null>(),
  documents: new Map<string, { languageId: string; version: number; text: string }>(),
};

/// Keep what `message` changes about the session. Called before the
/// server sees it, so that a message the server traps on is one the new
/// server is told about too.
function remember(message: Said): void {
  try {
    keep(message);
  } catch {
    // a message of the wrong shape, which the server refuses in turn
  }
}

function keep(message: Said): void {
  const params = message.params;
  switch (message.method) {
    case "initialize":
      session.initialize = message;
      break;
    case "initialized":
      session.initialized = true;
      break;
    case "sysml/files":
      for (const file of params?.files ?? []) {
        session.files.set(file.uri, file.text ?? null);
      }
      break;
    case "textDocument/didOpen": {
      const document = params.textDocument;
      session.documents.set(document.uri, {
        languageId: document.languageId,
        version: document.version,
        text: document.text,
      });
      break;
    }
    case "textDocument/didChange": {
      const document = session.documents.get(params.textDocument.uri);
      if (document) {
        for (const change of params.contentChanges ?? []) {
          document.text = changed(document.text, change);
        }
        document.version = params.textDocument.version;
      }
      break;
    }
    case "textDocument/didClose":
      session.documents.delete(params.textDocument.uri);
      break;
  }
}

/// `text` with one `contentChanges` entry applied, read as the server's
/// `apply_change` reads it: a position past the last line is the end,
/// a column past a line's end is that end, and a range written back to
/// front is the same range. Columns are UTF-16 code units, which is
/// what a JavaScript string is indexed by.
function changed(
  text: string,
  change: { range?: { start: Position; end: Position }; text: string }
): string {
  if (!change.range) {
    return change.text;
  }
  const a = offset(text, change.range.start);
  const b = offset(text, change.range.end);
  return text.slice(0, Math.min(a, b)) + change.text + text.slice(Math.max(a, b));
}

interface Position {
  line: number;
  character: number;
}

function offset(text: string, at: Position): number {
  let start = 0;
  for (let line = 0; line < at.line; line += 1) {
    const next = lineBreak(text, start);
    if (next === undefined) {
      return text.length;
    }
    start = next;
  }
  let end = start;
  while (end < text.length && text[end] !== "\n" && text[end] !== "\r") {
    end += 1;
  }
  return Math.min(start + at.character, end);
}

/// Where the line after the one starting at `start` starts, if any.
function lineBreak(text: string, start: number): number | undefined {
  for (let i = start; i < text.length; i += 1) {
    if (text[i] === "\n") {
      return i + 1;
    }
    if (text[i] === "\r") {
      return text[i + 1] === "\n" ? i + 2 : i + 1;
    }
  }
  return undefined;
}

/// Read `length` bytes from `at` in the module's memory. The view is
/// made afresh every time, since growing that memory -- which reading a
/// model does -- replaces the buffer every view was made against.
function decode(server: Server, at: number, length: number): string {
  return decoder.decode(new Uint8Array(server.memory.buffer, at, length));
}

/// The answers to one message, or what the panic it caused said.
function answer(server: Server, message: unknown): Said[] | { trapped: string } {
  const bytes = encoder.encode(JSON.stringify(message));
  try {
    const at = server.take(bytes.length);
    new Uint8Array(server.memory.buffer, at, bytes.length).set(bytes);
    const length = server.handle(bytes.length);
    return JSON.parse(decode(server, server.answers(), length));
  } catch (err) {
    // a panic reaches here as a trap with nothing in it, and the module
    // is the only place what it said still exists
    let said = 0;
    try {
      said = server.trap();
    } catch {
      // a module that traps on being asked has nothing more to say
    }
    return { trapped: said > 0 ? decode(server, server.answers(), said) : `${err}` };
  }
}

/// Give the server one message and post back whatever answers it.
function deliver(message: Said): void {
  remember(message);
  if (givenUp !== undefined || !server) {
    refuse(message, givenUp ?? "the language server is not running");
    return;
  }
  const answers = answer(server, message);
  if (!Array.isArray(answers)) {
    // the question that trapped is answered, so that the client's wait
    // for it ends, and the server is started again
    refuse(message, `\`${message.method}\` failed inside the server: ${answers.trapped}`);
    log(`the language server stopped, and is starting again: ${answers.trapped}`);
    restart();
    return;
  }
  for (const answer of answers) {
    postMessage(answer);
  }
}

/// Answer a question with an error, where `message` is one.
function refuse(message: Said, why: string): void {
  if (message.id !== undefined && message.id !== null && message.method) {
    postMessage({
      jsonrpc: "2.0",
      id: message.id,
      error: { code: -32603, message: why },
    });
  }
}

/// A new instance, told what the one that trapped had been told.
///
/// What it says back about the documents is posted, since it replaces
/// whatever the old one last published; the answer to the handshake is
/// not, since the client has had one. A server that traps again while
/// being brought back would only go on trapping, so then it is given up.
function restart(): void {
  if (!compiled) {
    return;
  }
  server = new WebAssembly.Instance(compiled, {}).exports as unknown as Server;
  const replay: Said[] = [];
  if (session.initialize) {
    replay.push(session.initialize);
    if (session.initialized) {
      replay.push({ method: "initialized", params: {} });
    }
  }
  if (session.files.size > 0) {
    const files = [...session.files].map(([uri, text]) => ({ uri, text }));
    replay.push({ method: "sysml/files", params: { files } });
  }
  for (const [uri, document] of session.documents) {
    replay.push({
      method: "textDocument/didOpen",
      params: { textDocument: { uri, ...document } },
    });
  }
  for (const message of replay) {
    const answers = answer(server, { jsonrpc: "2.0", ...message });
    if (!Array.isArray(answers)) {
      givenUp = `the language server stopped again while starting: ${answers.trapped}`;
      log(givenUp);
      server = undefined;
      return;
    }
    for (const said of answers) {
      if (said.id === undefined) {
        postMessage(said);
      }
    }
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
  compiled = await WebAssembly.compile(bytes);
  server = (await WebAssembly.instantiate(compiled, {})).exports as unknown as Server;
  for (const message of waiting.splice(0)) {
    deliver(message as Said);
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
  if (!server && givenUp === undefined) {
    waiting.push(said);
    return;
  }
  deliver(said);
};
