// The diagram preview: a webview beside the editor showing the SVG the
// language server draws for the active document, following unsaved edits.

import * as vscode from "vscode";
import { LanguageClient } from "vscode-languageclient/node";

type View = "definitions" | "internal" | "browser";

/// Whether a SysML or KerML document is what an editor is showing.
function isModel(editor: vscode.TextEditor | undefined): boolean {
  const language = editor?.document.languageId;
  return language === "sysml" || language === "kerml";
}

export class Preview {
  private panel: vscode.WebviewPanel | undefined;
  private uri: vscode.Uri | undefined;
  private view: View = "definitions";
  private element: string | undefined;
  private timer: NodeJS.Timeout | undefined;
  /// Closing the preview closes it: opening by itself must not undo
  /// that on the next keystroke in another file. Asking for it again
  /// says the reader has changed their mind.
  private dismissed = false;
  /// How often the server has answered that it has nothing yet. It is
  /// still opening the document when the preview opens with it, so the
  /// first answer is often none -- but not for ever.
  private waiting = 0;

  constructor(
    private readonly client: () => LanguageClient | undefined,
    subscriptions: vscode.Disposable[]
  ) {
    subscriptions.push(
      vscode.workspace.onDidChangeTextDocument((event) => {
        if (this.uri && event.document.uri.toString() === this.uri.toString()) {
          this.scheduleRender();
        }
      }),
      vscode.window.onDidChangeActiveTextEditor((editor) => {
        // the preview follows whichever SysML document is being edited
        if (editor && this.panel && isModel(editor)) {
          this.uri = editor.document.uri;
          this.scheduleRender();
        } else if (editor && !this.panel) {
          void this.followActiveEditor(editor);
        }
      })
    );
  }

  /// Open the preview for the document that is already being edited.
  ///
  /// The extension starts when a model is opened, so the document that
  /// started it is the active one before any event could say so -- and
  /// it has to wait for the server, which is what draws.
  async openForActiveEditor(): Promise<void> {
    await this.followActiveEditor(vscode.window.activeTextEditor);
  }

  /// Open the preview for a newly active model, where the reader asked
  /// for that to happen by itself and has not closed it since.
  private async followActiveEditor(
    editor: vscode.TextEditor | undefined
  ): Promise<void> {
    if (!editor || !isModel(editor) || this.panel || this.dismissed) {
      return;
    }
    const wanted = vscode.workspace
      .getConfiguration("sysml")
      .get<boolean>("preview.openAutomatically", true);
    if (wanted) {
      await this.open(editor.document.uri, "definitions");
    }
  }

  async open(uri: vscode.Uri, view: View, element?: string): Promise<void> {
    this.dismissed = false;
    this.waiting = 0;
    this.uri = uri;
    this.view = view;
    this.element = element;
    if (!this.panel) {
      this.panel = vscode.window.createWebviewPanel(
        "sysmlPreview",
        "SysML Preview",
        { viewColumn: vscode.ViewColumn.Beside, preserveFocus: true },
        { enableScripts: true }
      );
      this.panel.onDidDispose(() => {
        this.panel = undefined;
        this.dismissed = true;
      });
      this.panel.webview.onDidReceiveMessage((message) => {
        if (message.command === "setView") {
          this.view = message.view;
          this.element = message.element || undefined;
          this.scheduleRender();
        }
      });
    }
    await this.render();
  }

  private scheduleRender(): void {
    if (this.timer) {
      clearTimeout(this.timer);
    }
    this.timer = setTimeout(() => void this.render(), 300);
  }

  private async render(): Promise<void> {
    const client = this.client();
    if (!this.panel || !this.uri || !client) {
      return;
    }
    this.panel.title = `Preview ${this.uri.path.split("/").pop()}`;
    let svg: string;
    try {
      const result = await client.sendRequest<{ svg: string } | null>(
        "sysml/diagram",
        {
          uri: this.uri.toString(),
          view: this.view,
          element: this.element,
          layout: vscode.workspace
            .getConfiguration("sysml")
            .get<string>("diagram.layout", "elk"),
        }
      );
      if (result) {
        this.waiting = 0;
        svg = result.svg;
      } else {
        svg = "<p>Nothing to draw yet — the document may still be loading.</p>";
        if (this.waiting < 10) {
          this.waiting += 1;
          this.scheduleRender();
        }
      }
    } catch (error) {
      svg = `<p>Preview failed: ${String(error)}</p>`;
    }
    this.panel.webview.html = this.html(svg);
  }

  private html(svg: string): string {
    const internal = this.view === "internal";
    return `<!DOCTYPE html>
<html>
<head>
<meta charset="utf-8">
<style>
  body { padding: 0.5em; }
  #bar { display: flex; gap: 0.5em; align-items: center; margin-bottom: 0.5em;
         font-family: var(--vscode-font-family); font-size: 12px; }
  select, input {
    background: var(--vscode-input-background); color: var(--vscode-input-foreground);
    border: 1px solid var(--vscode-input-border, transparent); padding: 2px 4px;
  }
  #diagram { overflow: auto; }
  #diagram svg { max-width: none; }
</style>
</head>
<body>
<div id="bar">
  <select id="view">
    <option value="definitions"${this.view === "definitions" ? " selected" : ""}>Definitions</option>
    <option value="internal"${internal ? " selected" : ""}>Internal structure</option>
    <option value="browser"${this.view === "browser" ? " selected" : ""}>Tree</option>
  </select>
  <input id="element" placeholder="element name"
         value="${this.element ?? ""}" style="display:${internal ? "inline" : "none"}">
</div>
<div id="diagram">${svg}</div>
<script>
  const vscode = acquireVsCodeApi();
  const view = document.getElementById("view");
  const element = document.getElementById("element");
  function send() {
    element.style.display = view.value === "internal" ? "inline" : "none";
    vscode.postMessage({ command: "setView", view: view.value, element: element.value });
  }
  view.addEventListener("change", send);
  element.addEventListener("change", send);
</script>
</body>
</html>`;
  }
}
