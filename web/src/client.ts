// The language server, spoken to from a page.
//
// The same conversation the VSCode extension has -- `initialize`,
// `didOpen`, `didChange`, and the custom `sysml/diagram` that answers
// with an SVG -- held with a module running in a worker rather than a
// program running on a machine. There is no machine: this is a static
// site, and everything below the `postMessage` is the .wasm.
//
// The worker itself is the extension's, bundled from where it lives.
// See `scripts/build.mjs` for why.

/// One thing the server has to say about a document, in the Language
/// Server Protocol's own shape.
export interface Diagnostic {
  range: {
    start: { line: number; character: number };
    end: { line: number; character: number };
  };
  message: string;
  /// 1 error, 2 warning, 3 information, 4 hint.
  severity?: number;
}

/// One thing a client can do about a diagnostic: what to call it, and
/// the one edit that does it.
export interface Fix {
  title: string;
  range: Diagnostic["range"];
  text: string;
}

/// What a drawing can be of. `definitions` is the definitions in the
/// document and the relationships between them, `internal` is one
/// element's parts and how they are connected, `browser` is the
/// membership tree.
export type View = "definitions" | "internal" | "browser";

/// A code action as the protocol sends it. Only the ones that carry a
/// single edit to this document are any use here.
interface Offered {
  title: string;
  edit?: {
    changes?: Record<string, { range: Diagnostic["range"]; newText: string }[]>;
  };
}

/// A message on the way back. Only `result`/`error` makes one an answer:
/// the server may send a request of its own, and that also carries an id.
interface Answer {
  id?: number;
  method?: string;
  result?: unknown;
  error?: { message: string };
  params?: Record<string, unknown>;
}

/// How long to wait for an answer before giving up on it.
///
/// The module is single-threaded and answers every message in the order
/// it arrives, so a request that has not come back by now is one that
/// never will -- a panic reaches the worker as a trap, and the worker
/// logs it and returns with no answers at all. Without this the page
/// would sit on "drawing" for the rest of the session.
const PATIENCE = 30_000;

/// The conversation with one language server.
export class Client {
  private readonly worker: Worker;
  private readonly pending = new Map<number, (answer: Answer) => void>();
  private next = 1;
  /// What the server was last told each document is at, since
  /// `didChange` is refused without a version later than the last.
  private readonly versions = new Map<string, number>();

  /// What the server publishes about a document whenever it changes.
  onDiagnostics: (uri: string, said: Diagnostic[]) => void = () => {};
  /// What the server logs -- including, at the end of a bad session,
  /// what a panic said.
  onLog: (said: string) => void = () => {};

  private constructor(worker: Worker) {
    this.worker = worker;
    worker.onmessage = (event: MessageEvent) => this.receive(event.data as Answer);
  }

  /// Fetch the module, boot it in a worker, and shake hands.
  ///
  /// The standard library is inside the module -- 94 files it reads and
  /// resolves before it answers this -- so the promise is the half
  /// second or so that takes, and a keystroke after it is milliseconds.
  static async start(module: string, worker: string): Promise<Client> {
    const client = new Client(new Worker(worker));
    const answered = await fetch(module);
    if (!answered.ok) {
      throw new Error(`the language server did not load: ${answered.status}`);
    }
    const wasm = await answered.arrayBuffer();
    // transferred rather than copied: it is six megabytes
    client.worker.postMessage({ wasm }, [wasm]);
    await client.ask("initialize", {
      capabilities: {},
      workspaceFolders: [{ uri: "file:///playground", name: "playground" }],
    });
    client.tell("initialized", {});
    return client;
  }

  /// Open a document. Its diagnostics follow.
  open(uri: string, text: string): void {
    this.versions.set(uri, 1);
    this.tell("textDocument/didOpen", {
      textDocument: { uri, languageId: "sysml", version: 1, text },
    });
  }

  /// Say what a document says now. Whole-document changes, because that
  /// is what a textarea knows how to report.
  change(uri: string, text: string): void {
    const version = (this.versions.get(uri) ?? 1) + 1;
    this.versions.set(uri, version);
    this.tell("textDocument/didChange", {
      textDocument: { uri, version },
      contentChanges: [{ text }],
    });
  }

  /// What the server offers about whatever is written over `range`.
  ///
  /// The standard `textDocument/codeAction`, which for this server is
  /// what a name that resolved to nothing might have meant. Asked rather
  /// than published: the walk behind it is over every declared name, so
  /// it belongs on the click that wants it.
  async fixes(uri: string, range: Diagnostic["range"]): Promise<Fix[]> {
    const offered = (await this.ask("textDocument/codeAction", {
      textDocument: { uri },
      range,
      // the server reads the range rather than this, but the protocol
      // says a context is sent, so one is
      context: { diagnostics: [] },
    })) as Offered[] | null;
    return (offered ?? []).flatMap((action) => {
      const edit = action.edit?.changes?.[uri]?.[0];
      return edit ? [{ title: action.title, range: edit.range, text: edit.newText }] : [];
    });
  }

  /// Draw a document, as a standalone SVG. Nothing to draw is `undefined`.
  async diagram(of: {
    uri: string;
    view: View;
    element?: string;
  }): Promise<string | undefined> {
    const drawn = (await this.ask("sysml/diagram", {
      uri: of.uri,
      view: of.view,
      element: of.element,
      scope: "file",
    })) as { svg?: string } | null;
    return drawn?.svg;
  }

  private tell(method: string, params: unknown): void {
    this.worker.postMessage({ jsonrpc: "2.0", method, params });
  }

  private ask(method: string, params: unknown): Promise<unknown> {
    const id = this.next;
    this.next += 1;
    return new Promise((resolve, reject) => {
      const patience = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`the language server did not answer \`${method}\``));
      }, PATIENCE);
      this.pending.set(id, (answer) => {
        clearTimeout(patience);
        if (answer.error) {
          reject(new Error(answer.error.message));
        } else {
          resolve(answer.result ?? null);
        }
      });
      this.worker.postMessage({ jsonrpc: "2.0", id, method, params });
    });
  }

  private receive(message: Answer): void {
    if (
      typeof message?.id === "number" &&
      (message.result !== undefined || message.error !== undefined)
    ) {
      const settle = this.pending.get(message.id);
      this.pending.delete(message.id);
      settle?.(message);
      return;
    }
    if (message?.method === "textDocument/publishDiagnostics") {
      const params = message.params as { uri: string; diagnostics?: Diagnostic[] };
      this.onDiagnostics(params.uri, params.diagnostics ?? []);
    } else if (message?.method === "window/logMessage") {
      this.onLog(String((message.params as { message?: unknown })?.message ?? ""));
    }
  }
}
