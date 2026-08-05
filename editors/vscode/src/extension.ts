// VSCode client for the SysML v2 language server (`sysml-lsp`).
//
// The server does the work -- diagnostics, completion, navigation, rename,
// formatting -- over stdio; this extension only launches it and passes the
// standard-library path along so library names resolve.

import * as fs from "fs";
import * as path from "path";
import * as vscode from "vscode";
import {
  LanguageClient,
  LanguageClientOptions,
  ServerOptions,
} from "vscode-languageclient/node";
import { Preview } from "./preview";

let client: LanguageClient | undefined;

/// The server to run: the explicit setting, else the binary bundled into
/// the extension by `make vscode`, else `sysml-lsp` on PATH.
function serverCommand(context: vscode.ExtensionContext): string {
  const configured = vscode.workspace
    .getConfiguration("sysml")
    .get<string>("server.path");
  if (configured) {
    return configured;
  }
  const executable = process.platform === "win32" ? "sysml-lsp.exe" : "sysml-lsp";
  const bundled = context.asAbsolutePath(path.join("server", executable));
  if (fs.existsSync(bundled)) {
    return bundled;
  }
  return "sysml-lsp";
}

/// The standard library: the explicit setting, else the copy bundled into
/// the extension, else whatever SYSML_LIBRARY_PATH says (read server-side).
function libraryPath(context: vscode.ExtensionContext): string | undefined {
  const configured = vscode.workspace
    .getConfiguration("sysml")
    .get<string>("library.path");
  if (configured) {
    return configured;
  }
  const bundled = context.asAbsolutePath(path.join("library", "sysml.library"));
  return fs.existsSync(bundled) ? bundled : undefined;
}

function serverOptions(context: vscode.ExtensionContext): ServerOptions {
  return { command: serverCommand(context), args: [] };
}

function clientOptions(context: vscode.ExtensionContext): LanguageClientOptions {
  const library = libraryPath(context);
  const dot = vscode.workspace
    .getConfiguration("sysml")
    .get<string>("diagram.dot", "dot");
  return {
    // no scheme filter: untitled buffers get language support too
    documentSelector: [{ language: "sysml" }, { language: "kerml" }],
    initializationOptions: {
      ...(library ? { libraryPath: library } : {}),
      dotCommand: dot,
    },
  };
}

async function start(context: vscode.ExtensionContext): Promise<void> {
  client = new LanguageClient(
    "sysml",
    "SysML v2 Language Server",
    serverOptions(context),
    clientOptions(context)
  );
  try {
    await client.start();
  } catch {
    client = undefined;
    const command = serverCommand(context);
    const choice = await vscode.window.showErrorMessage(
      `Cannot start \`${command}\`. Run \`make vscode\` in the repository, ` +
        "or build with `cargo build --release -p sysml-lsp` and set `sysml.server.path`.",
      "Open Settings"
    );
    if (choice === "Open Settings") {
      vscode.commands.executeCommand(
        "workbench.action.openSettings",
        "sysml.server.path"
      );
    }
  }
}

export async function activate(
  context: vscode.ExtensionContext
): Promise<void> {
  const preview = new Preview(() => client, context.subscriptions);
  context.subscriptions.push(
    vscode.commands.registerCommand("sysml.restartServer", async () => {
      if (client) {
        await client.stop();
        client = undefined;
      }
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
}

export async function deactivate(): Promise<void> {
  if (client) {
    await client.stop();
    client = undefined;
  }
}
