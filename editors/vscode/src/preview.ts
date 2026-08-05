// The diagram preview: a webview beside the editor showing the SVG the
// language server draws for the active document, following unsaved edits.

import * as vscode from "vscode";
import { LanguageClient } from "vscode-languageclient/node";

type View = "definitions" | "internal" | "browser";

export class Preview {
  private panel: vscode.WebviewPanel | undefined;
  private uri: vscode.Uri | undefined;
  private view: View = "definitions";
  private element: string | undefined;
  private timer: NodeJS.Timeout | undefined;

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
        if (
          editor &&
          this.panel &&
          (editor.document.languageId === "sysml" ||
            editor.document.languageId === "kerml")
        ) {
          this.uri = editor.document.uri;
          this.scheduleRender();
        }
      })
    );
  }

  async open(uri: vscode.Uri, view: View, element?: string): Promise<void> {
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
            .get<string>("diagram.layout", "graphviz"),
        }
      );
      svg = result
        ? result.svg
        : "<p>Nothing to draw yet — the document may still be loading.</p>";
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
