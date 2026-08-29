// The diagram preview: a webview beside the editor showing the SVG the
// language server draws for the active document, following unsaved edits.

import * as vscode from "vscode";
import { LanguageClient } from "vscode-languageclient/node";
import { page } from "./page";

type View = "definitions" | "internal" | "browser";

/// What the preview is showing: a drawing, or a line of text where there
/// is none to show.
type Drawing = { kind: "svg" | "message"; body: string };

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
  /// The drawing on screen, which is also the one a save writes out.
  private drawing: Drawing | undefined;

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
      // the page is written once and the drawings are posted into it, so
      // that a keystroke redraws without throwing away where the reader
      // had scrolled to and how far in they had zoomed
      this.panel.webview.html = page();
      this.panel.onDidDispose(() => {
        this.panel = undefined;
        this.dismissed = true;
      });
      this.panel.webview.onDidReceiveMessage((message) =>
        this.receive(message)
      );
      // a hidden panel is torn down and rebuilt when it comes back, and
      // the rebuilt page starts empty until it is drawn into again
      this.panel.onDidChangeViewState(() => {
        if (this.panel?.visible) {
          this.scheduleRender();
        }
      });
    }
    await this.render();
  }

  private receive(message: {
    command: string;
    view?: View;
    element?: string;
    png?: string;
  }): void {
    if (message.command === "ready") {
      void this.render();
    } else if (message.command === "setView") {
      this.view = message.view ?? "definitions";
      this.element = message.element || undefined;
      this.scheduleRender();
    } else if (message.command === "save") {
      void this.save(message.png);
    }
  }

  /// Write the drawing to a file the reader names.
  ///
  /// The format follows the name they give it: the SVG is what the server
  /// drew, and a PNG is the webview's rendering of that same drawing,
  /// which is why it arrives with the request rather than being made here.
  private async save(png: string | undefined): Promise<void> {
    if (!this.uri || this.drawing?.kind !== "svg") {
      await vscode.window.showWarningMessage("There is no diagram to save.");
      return;
    }
    const file = this.uri.path.split("/").pop() ?? "diagram";
    const stem = file.replace(/\.(sysml|kerml)$/i, "");
    const about =
      this.view === "internal" && this.element ? this.element : this.view;
    // a qualified name is a legal thing to ask for and an illegal thing
    // to call a file, on Windows at least
    const named = `${stem}-${about}`.replace(/[^\w.-]+/g, "-");
    // an untitled buffer is nowhere, so the folder that is open stands in
    const beside =
      this.uri.scheme === "file"
        ? vscode.Uri.joinPath(this.uri, "..")
        : vscode.workspace.workspaceFolders?.[0]?.uri;
    const target = await vscode.window.showSaveDialog({
      title: "Save diagram",
      ...(beside
        ? { defaultUri: vscode.Uri.joinPath(beside, `${named}.svg`) }
        : {}),
      filters: { "SVG image": ["svg"], "PNG image": ["png"] },
    });
    if (!target) {
      return;
    }
    if (target.path.toLowerCase().endsWith(".png")) {
      if (!png) {
        await vscode.window.showErrorMessage(
          "The preview could not turn this diagram into a PNG. Save it as SVG instead."
        );
        return;
      }
      await vscode.workspace.fs.writeFile(target, Buffer.from(png, "base64"));
    } else {
      await vscode.workspace.fs.writeFile(
        target,
        Buffer.from(this.drawing.body, "utf8")
      );
    }
    const name = target.path.split("/").pop();
    const choice = await vscode.window.showInformationMessage(
      `Saved ${name}`,
      "Open"
    );
    if (choice === "Open") {
      await vscode.commands.executeCommand("vscode.open", target);
    }
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
    let drawing: Drawing;
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
        drawing = { kind: "svg", body: result.svg };
      } else {
        drawing = {
          kind: "message",
          body: "Nothing to draw yet — the document may still be loading.",
        };
        if (this.waiting < 10) {
          this.waiting += 1;
          this.scheduleRender();
        }
      }
    } catch (error) {
      drawing = { kind: "message", body: `Preview failed: ${String(error)}` };
    }
    this.drawing = drawing;
    await this.panel.webview.postMessage({
      command: "draw",
      kind: drawing.kind,
      body: drawing.body,
      view: this.view,
      element: this.element ?? "",
    });
  }
}
