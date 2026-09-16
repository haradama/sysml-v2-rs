// VSCode client for the SysML v2 language server.
//
// The server is a WebAssembly module in a worker: one file for every
// machine VSCode runs on, and for the browser, where vscode.dev and
// github.dev have no machine underneath at all. Nothing is spawned and
// nothing is installed -- and nothing on the server's side can read a
// file, so this extension does the reading as well as the talking.

import * as vscode from "vscode";
import {
  LanguageClient,
  LanguageClientOptions,
} from "vscode-languageclient/browser";
import { Preview } from "./preview";

let client: LanguageClient | undefined;
let server: Worker | undefined;

/// Every file a SysML model is written across.
const MODELS = "**/*.{sysml,kerml}";

/// A file as the server takes it: the URI it is known by, and what it
/// says -- or null, for one that is gone.
type Handed = { uri: string; text: string | null };

/// Whether a name is a model file's.
function isModel(name: string): boolean {
  return name.endsWith(".sysml") || name.endsWith(".kerml");
}

/// What a file says, or nothing where it cannot be read: a link to
/// nowhere, a file the workspace lists and the filesystem has not.
async function read(uri: vscode.Uri): Promise<string | null> {
  try {
    return new TextDecoder().decode(await vscode.workspace.fs.readFile(uri));
  } catch {
    return null;
  }
}

/// The files, with what each of them says.
async function handed(uris: readonly vscode.Uri[]): Promise<Handed[]> {
  const files = await Promise.all(
    uris.map(async (uri) => ({ uri: uri.toString(), text: await read(uri) }))
  );
  return files.filter((file) => file.text !== null);
}

/// The model files directly in a directory.
async function filesIn(dir: vscode.Uri): Promise<vscode.Uri[]> {
  try {
    const entries = await vscode.workspace.fs.readDirectory(dir);
    return entries
      .filter(([name, kind]) => kind === vscode.FileType.File && isModel(name))
      .map(([name]) => vscode.Uri.joinPath(dir, name));
  } catch {
    // a buffer can stand where no directory does: a file the editor
    // holds and has never written
    return [];
  }
}

/// ... and the model files in the tree below it.
async function filesUnder(dir: vscode.Uri): Promise<vscode.Uri[]> {
  let entries: [string, vscode.FileType][];
  try {
    entries = await vscode.workspace.fs.readDirectory(dir);
  } catch {
    return [];
  }
  const found: vscode.Uri[] = [];
  for (const [name, kind] of entries) {
    const at = vscode.Uri.joinPath(dir, name);
    if (kind === vscode.FileType.Directory) {
      found.push(...(await filesUnder(at)));
    } else if (kind === vscode.FileType.File && isModel(name)) {
      found.push(at);
    }
  }
  return found;
}

/// The model files beside the documents that are open already and are
/// in no workspace folder: a document is read in the company it was
/// written in, and `onDidOpenTextDocument` never fires for the tabs
/// that were already there when the window came up.
async function besideOpenDocuments(): Promise<vscode.Uri[]> {
  const directories = new Map<string, vscode.Uri>();
  for (const document of vscode.workspace.textDocuments) {
    const uri = document.uri;
    if (!isModel(uri.path) || vscode.workspace.getWorkspaceFolder(uri)) {
      continue;
    }
    const dir = vscode.Uri.joinPath(uri, "..");
    directories.set(dir.toString(), dir);
  }
  const found = await Promise.all([...directories.values()].map(filesIn));
  return found.flat();
}

/// The standard library to resolve names against: the directory the
/// setting names, or nothing, which leaves the server the copy built
/// into the module.
function libraryDirectory(): vscode.Uri | undefined {
  const configured = vscode.workspace
    .getConfiguration("sysml")
    .get<string>("library.path");
  if (!configured) {
    return undefined;
  }
  // a setting written as a path is a path on the machine the window is
  // on; one written as a URI is wherever it says
  return configured.includes("://")
    ? vscode.Uri.parse(configured)
    : vscode.Uri.file(configured);
}

/// What the project is not to read: the setting, with anything that
/// names one place said as the URI of that place, since a workspace
/// that is not a filesystem has no other way to say so.
function excluded(): string[] {
  return vscode.workspace
    .getConfiguration("sysml")
    .get<string[]>("workspace.exclude", [])
    .map((entry) => {
      const absolute = entry.startsWith("/") || /^[a-zA-Z]:[\\/]/.test(entry);
      return absolute ? vscode.Uri.file(entry).toString() : entry;
    });
}

/// Hand the server a change to the files it reads.
///
/// A file written while the server is still starting is a notification
/// to a client that is not running, which throws. Nothing to do about
/// it: the handshake reads the file for itself.
async function provide(files: Handed[]): Promise<void> {
  if (!client || files.length === 0) {
    return;
  }
  try {
    await client.sendNotification("sysml/files", { files });
  } catch {
    // the handshake will carry it
  }
}

async function clientOptions(): Promise<LanguageClientOptions> {
  const width = vscode.workspace
    .getConfiguration("sysml")
    .get<number>("format.width", 100);
  const skin = vscode.workspace
    .getConfiguration("sysml")
    .get<unknown>("diagram.skin", "default");
  // A file and an unsaved buffer are documents the server can speak
  // for, and so is a file in a workspace nobody has checked out --
  // github.dev opens a repository under a scheme of its own. The other
  // schemes a window shows are not: a `git:` document is some earlier
  // revision of a file, and handing it over declares that revision's
  // names alongside the working copy's, in the same project, as though
  // the model held both.
  const schemes = ["file", "untitled", "vscode-vfs"];
  const languages = ["sysml", "kerml"];
  // The files the project is made of, sent in the handshake: a server
  // told afterwards would diagnose the first document that opened
  // against a project of nothing.
  //
  // Everything the editor can see, `workspace.exclude` included. Which
  // of them the project reads is the server's decision, as it was
  // before -- and a document opened from inside an excluded directory
  // still needs its neighbours.
  const library = libraryDirectory();
  const files = await handed([
    ...(await vscode.workspace.findFiles(MODELS)),
    ...(await besideOpenDocuments()),
    ...(library ? await filesUnder(library) : []),
  ]);
  const exclude = excluded();
  return {
    documentSelector: schemes.flatMap((scheme) =>
      languages.map((language) => ({ scheme, language }))
    ),
    initializationOptions: {
      files,
      ...(library ? { libraryPath: library.toString() } : {}),
      ...(exclude.length > 0 ? { excludePaths: exclude } : {}),
      formatWidth: width,
      skin,
    },
  };
}

/// The module, and the worker to run it in. The worker is handed the
/// bytes rather than left to fetch them, since the client is the side
/// that can read an extension's own files in every host VSCode has.
async function spawn(context: vscode.ExtensionContext): Promise<Worker> {
  const at = vscode.Uri.joinPath(context.extensionUri, "out", "server.worker.js");
  const worker = new Worker(at.toString(true));
  try {
    worker.postMessage({ wasm: await moduleBytes(context) });
  } catch (err) {
    // a worker with nothing to run is a worker to let go of
    worker.terminate();
    throw err;
  }
  return worker;
}

/// The module's bytes, read whichever way this host has.
///
/// An extension's own files are a `file:` URI in a desktop window and an
/// `https:` one in a browser tab, which `workspace.fs` and `fetch` read
/// respectively. Asking both is cheaper than deciding which host this
/// is.
async function moduleBytes(context: vscode.ExtensionContext): Promise<Uint8Array> {
  const at = vscode.Uri.joinPath(context.extensionUri, "server", "sysml-lsp.wasm");
  try {
    return await vscode.workspace.fs.readFile(at);
  } catch {
    const answer = await fetch(at.toString(true));
    if (!answer.ok) {
      throw new Error(`${at.toString(true)}: ${answer.status}`);
    }
    return new Uint8Array(await answer.arrayBuffer());
  }
}

async function start(context: vscode.ExtensionContext): Promise<void> {
  try {
    server = await spawn(context);
  } catch {
    const choice = await vscode.window.showErrorMessage(
      "The SysML language server is not in this extension. Run `make vscode` " +
        "in the repository to build it, or install a released .vsix.",
      "Open Logs"
    );
    if (choice === "Open Logs") {
      void vscode.commands.executeCommand("workbench.action.showLogs");
    }
    return;
  }
  client = new LanguageClient(
    "sysml",
    "SysML v2 Language Server",
    await clientOptions(),
    server
  );
  await client.start();
}

async function stop(): Promise<void> {
  if (client) {
    await client.stop();
    client = undefined;
  }
  // the client takes the worker with it, but a client that never
  // started leaves one behind
  server?.terminate();
  server = undefined;
}

export async function activate(
  context: vscode.ExtensionContext
): Promise<void> {
  const preview = new Preview(() => client, context.subscriptions);
  // a model file written, renamed or deleted outside the editor belongs
  // to the project too, and only the client can see it happen
  const watcher = vscode.workspace.createFileSystemWatcher(MODELS);
  const changed = async (uri: vscode.Uri) => {
    await provide(await handed([uri]));
  };
  context.subscriptions.push(
    watcher,
    watcher.onDidCreate(changed),
    watcher.onDidChange(changed),
    watcher.onDidDelete((uri) => {
      void provide([{ uri: uri.toString(), text: null }]);
    }),
    // the same for a document opened from outside the workspace
    // folders, which would otherwise report every name its other half
    // declares as unresolved
    vscode.workspace.onDidOpenTextDocument(async (document) => {
      const uri = document.uri;
      if (!isModel(uri.path) || vscode.workspace.getWorkspaceFolder(uri)) {
        return;
      }
      await provide(await handed(await filesIn(vscode.Uri.joinPath(uri, ".."))));
    }),
    vscode.commands.registerCommand("sysml.restartServer", async () => {
      await stop();
      await start(context);
    }),
    vscode.commands.registerCommand("sysml.showPreview", async () => {
      const editor = vscode.window.activeTextEditor;
      if (!editor) {
        return;
      }
      await preview.open(editor.document.uri, "definitions");
    })
  );
  await start(context);
  // the server draws the preview, so it has to be running first
  await preview.openForActiveEditor();
}

export async function deactivate(): Promise<void> {
  await stop();
}
