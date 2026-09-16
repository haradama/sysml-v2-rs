// The page: a model on the left, the drawing of it on the right, and a
// language server in a worker between them.
//
// Everything here is the front end. What a model means -- whether it
// parses, what its names resolve to, which of the specification's
// constraints it violates, and where the boxes go -- is the .wasm's
// answer, asked for over the Language Server Protocol in `client.ts`.

import { Client, type Diagnostic, type Fix, type View } from "./client";
import { EXAMPLES } from "./examples";
import { highlighter } from "./highlight";

/// The document the page edits. The server has no filesystem, so a URI
/// is a name and nothing more -- see `Files::Handed` in `sysmlv2-lsp`.
const URI = "file:///playground/model.sysml";

/// How long to let typing settle before redrawing. Long enough that a
/// word typed at speed is one drawing rather than six, short enough
/// that stopping to look feels immediate.
const SETTLE = 250;

const LEAST = 0.1;
const MOST = 8;
const STEP = 1.2;

function need<T extends HTMLElement>(id: string): T {
  const found = document.getElementById(id);
  if (!found) {
    throw new Error(`the page has no #${id}`);
  }
  return found as T;
}

const source = need<HTMLTextAreaElement>("source");
const painted = need<HTMLPreElement>("painted");
const gutter = need<HTMLDivElement>("gutter");
const problems = need<HTMLUListElement>("problems");
const chooseExample = need<HTMLSelectElement>("example");
const chooseView = need<HTMLSelectElement>("view");
const element = need<HTMLInputElement>("element");
const pane = need<HTMLDivElement>("pane");
const sizer = need<HTMLDivElement>("sizer");
const canvas = need<HTMLDivElement>("canvas");
const readout = need<HTMLSpanElement>("zoom");
const state = need<HTMLSpanElement>("state");
const split = need<HTMLDivElement>("split");
const saving = need<HTMLDivElement>("saving");
const saveName = need<HTMLInputElement>("save-name");
const saveKind = need<HTMLSelectElement>("save-kind");
const panes = split.parentElement as HTMLElement;

// --------------------------------------------------------------- panes

/// How little of the window either pane may be left with. Far enough
/// over for one of them to be most of the window, not so far that the
/// other is a line nobody can aim at to drag it back.
const NARROWEST = 12;

/// How much of the window the model has, so that the arrow keys can move
/// it by a step without reading it back off the page -- and so that a
/// window with no width yet cannot turn it into a `NaN`.
let share = 50;

/// Give the model `part` per cent of the window and the drawing the rest.
function divide(part: number): void {
  if (!Number.isFinite(part)) {
    return;
  }
  share = Math.min(100 - NARROWEST, Math.max(NARROWEST, part));
  panes.style.setProperty("--left", `${share}%`);
  split.setAttribute("aria-valuenow", String(Math.round(share)));
  // a drawing nobody has zoomed is fitted to the pane it is in, and the
  // pane just changed size
  settle();
  apply();
}

/// Where the pointer is, as a share of the window the two panes are in.
function shareAt(x: number): number {
  const box = panes.getBoundingClientRect();
  return ((x - box.left) / box.width) * 100;
}

// ------------------------------------------------------------ colouring

/// What paints the model, once the grammar has arrived. Until it does
/// -- and if it never does -- the textarea shows its own text.
let paint: ((source: string) => string | undefined) | undefined;

/// Number the lines, and make room for as many digits as there are.
///
/// One entry per line and one newline more, the same shape the painted
/// layer has, so the three layers are the same height and a number stays
/// beside the line it counts.
function number(): void {
  const lines = source.value.split("\n").length;
  gutter.textContent = `${Array.from({ length: lines }, (_, at) => at + 1).join("\n")}\n`;
  const width = Math.max(2, String(lines).length);
  source.parentElement?.style.setProperty("--gutter", `${width}ch`);
}

/// Colour what is in the editor, and keep the two layers level.
///
/// Not on the redraw's timer: a quarter of a second is nothing to wait
/// for a diagram and a long time to watch the word you just typed sit
/// there uncoloured.
function repaint(): void {
  if (!paint) {
    return;
  }
  const html = paint(source.value);
  if (html === undefined) {
    // too long to colour; the text shows for itself again
    source.parentElement?.classList.remove("painted");
    painted.textContent = "";
    return;
  }
  painted.innerHTML = html;
  source.parentElement?.classList.add("painted");
  follow();
}

/// The layers under the text scroll with it and never by themselves.
/// The numbers follow it up and down but not sideways.
function follow(): void {
  painted.scrollTop = source.scrollTop;
  painted.scrollLeft = source.scrollLeft;
  gutter.scrollTop = source.scrollTop;
}

/// Replace what is between `from` and `to`, the way a person would.
///
/// A textarea keeps its own undo history, and nothing a page writes into
/// `value` joins it -- assigning clears it outright, and `setRangeText`
/// leaves it alone but is not in it. `execCommand` is deprecated and is
/// still the only edit a page can make that a reader can take back, so
/// it is what applying a fix and indenting with Tab both go through, and
/// the direct write is what a browser that refuses gets.
function replace(from: number, to: number, text: string): void {
  source.focus();
  source.setSelectionRange(from, to);
  let joined = false;
  try {
    joined = document.execCommand("insertText", false, text);
  } catch {
    // a host with no `execCommand` at all
  }
  if (!joined) {
    source.setRangeText(text, from, to, "end");
  }
  // `execCommand` raises an input event of its own and `setRangeText`
  // does not, so this runs once either way and twice for neither
  edited();
}

/// The model changed: number and colour it now, draw it when the typing
/// settles.
function edited(): void {
  number();
  repaint();
  follow();
  schedule();
}

// ---------------------------------------------------------------- zoom

/// The size the drawing was drawn at, which is what a zoom is a factor of.
let natural = { width: 0, height: 0 };
let scale = 1;
/// Whether the reader has chosen a zoom. Until they have, a drawing too
/// big for the pane is scaled down to it; afterwards what they chose is
/// kept, or the diagram would jump under them at every keystroke.
let chosen = false;

function apply(): void {
  canvas.style.transform = `scale(${scale})`;
  sizer.style.width = `${natural.width * scale}px`;
  sizer.style.height = `${natural.height * scale}px`;
  readout.textContent = `${Math.round(scale * 100)}%`;
}

/// Zoom so that whatever is under (x, y) on screen stays under it.
function zoomAt(wanted: number, x: number, y: number): void {
  const next = Math.min(MOST, Math.max(LEAST, wanted));
  const box = pane.getBoundingClientRect();
  const held = { x: x - box.left + pane.scrollLeft, y: y - box.top + pane.scrollTop };
  const ratio = next / scale;
  scale = next;
  chosen = true;
  apply();
  pane.scrollLeft = held.x * ratio - (x - box.left);
  pane.scrollTop = held.y * ratio - (y - box.top);
}

/// Zoom about the middle of the pane, which is what a button means.
function zoomBy(factor: number): void {
  const box = pane.getBoundingClientRect();
  zoomAt(scale * factor, box.left + box.width / 2, box.top + box.height / 2);
}

function fit(): void {
  if (!natural.width || !natural.height) {
    return;
  }
  const room = 24;
  scale = Math.min(
    MOST,
    Math.max(
      LEAST,
      Math.min(
        (pane.clientWidth - room) / natural.width,
        (pane.clientHeight - room) / natural.height
      )
    )
  );
  chosen = true;
  apply();
  pane.scrollLeft = 0;
  pane.scrollTop = 0;
}

/// A drawing nobody has zoomed is shown whole if it fits and scaled down
/// to the pane if it does not. Never scaled up: a two-box diagram
/// stretched across a monitor reads as a mistake.
function settle(): void {
  if (chosen || !natural.width || !natural.height) {
    return;
  }
  const room = 24;
  scale = Math.max(
    LEAST,
    Math.min(
      1,
      (pane.clientWidth - room) / natural.width,
      (pane.clientHeight - room) / natural.height
    )
  );
}

// ------------------------------------------------------------- the link
//
// The model travels in the address bar, the way PlantUML's does, so a
// link is the whole thing a reader needs. It is a fragment, which a
// browser never sends: sharing one is between the two people holding it
// and no server, this one included.

function pack(said: string): string {
  const bytes = new TextEncoder().encode(said);
  let binary = "";
  for (const byte of bytes) {
    binary += String.fromCharCode(byte);
  }
  return btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

function unpack(said: string): string {
  const padded = said.replace(/-/g, "+").replace(/_/g, "/");
  const binary = atob(padded);
  const bytes = Uint8Array.from(binary, (letter) => letter.charCodeAt(0));
  return new TextDecoder().decode(bytes);
}

/// Put what is on screen into the address bar, without adding to the
/// history: every keystroke passes through here.
function remember(): void {
  const held = new URLSearchParams();
  held.set("view", chooseView.value);
  if (chooseView.value === "internal" && element.value) {
    held.set("of", element.value);
  }
  held.set("m", pack(source.value));
  history.replaceState(null, "", `#${held}`);
}

/// What the address bar was opened with, if it was opened with anything.
function opened(): { source: string; view: View; element: string } | undefined {
  const hash = location.hash.replace(/^#/, "");
  if (!hash) {
    return undefined;
  }
  const held = new URLSearchParams(hash);
  const model = held.get("m");
  if (model === null) {
    return undefined;
  }
  try {
    const view = held.get("view");
    return {
      source: unpack(model),
      view: view === "internal" || view === "browser" ? view : "definitions",
      element: held.get("of") ?? "",
    };
  } catch {
    // a link that arrived truncated, which is not worth an error page
    return undefined;
  }
}

// ------------------------------------------------------------ the model

let client: Client | undefined;
/// Whether a drawing is being waited for, and whether the model changed
/// while it was. The module answers one message at a time, so requests
/// are not piled onto it.
let drawing = false;
let again = false;
let timer: ReturnType<typeof setTimeout> | undefined;
/// What the server was last told the document says. Choosing another
/// view is not an edit, and telling it the same text again would have it
/// parse and resolve the model afresh to draw what it already holds.
let sent: string | undefined;

function say(said: string, trouble = false): void {
  state.textContent = said;
  state.classList.toggle("trouble", trouble);
}

function schedule(): void {
  clearTimeout(timer);
  timer = setTimeout(() => void draw(), SETTLE);
}

async function draw(): Promise<void> {
  if (!client) {
    return;
  }
  if (drawing) {
    again = true;
    return;
  }
  drawing = true;
  remember();
  try {
    if (source.value !== sent) {
      sent = source.value;
      client.change(URI, sent);
    }
    const svg = await client.diagram({
      uri: URI,
      view: chooseView.value as View,
      element: element.value || undefined,
    });
    show(svg);
  } catch (why) {
    say(`${why instanceof Error ? why.message : why}`, true);
  } finally {
    drawing = false;
    if (again) {
      again = false;
      void draw();
    }
  }
}

/// Put a drawing on the page, or say why there is none.
function show(svg: string | undefined): void {
  if (!svg) {
    canvas.replaceChildren(nothing(nothingToDraw()));
    natural = { width: 0, height: 0 };
    apply();
    return;
  }
  // The renderer escapes every name it writes, and the page's
  // Content-Security-Policy allows no inline script -- so a model
  // carrying a `<script>` into this window would have to get past both.
  canvas.innerHTML = svg;
  const drawn = canvas.querySelector("svg");
  if (!drawn) {
    canvas.replaceChildren(nothing("The drawing did not come back as an SVG."));
    natural = { width: 0, height: 0 };
    apply();
    return;
  }
  natural = {
    width: parseFloat(drawn.getAttribute("width") ?? "") || drawn.clientWidth,
    height: parseFloat(drawn.getAttribute("height") ?? "") || drawn.clientHeight,
  };
  settle();
  apply();
}

/// Why there is no drawing, which is nearly always the element box: an
/// internal view is of one element, and until it names one there is
/// nothing for the server to draw.
function nothingToDraw(): string {
  if (chooseView.value !== "internal") {
    return "Nothing to draw yet.";
  }
  return element.value
    ? `Nothing here is called \u201c${element.value}\u201d.`
    : "Name the element this view is of.";
}

function nothing(said: string): HTMLParagraphElement {
  const line = document.createElement("p");
  line.className = "nothing";
  line.textContent = said;
  return line;
}

/// Where a line and a character land in the text.
///
/// The protocol counts UTF-16 code units from the start of a line, which
/// is what a JavaScript string is made of, so this is arithmetic rather
/// than a conversion.
function offsetOf(where: Diagnostic["range"]["start"]): number {
  const lines = source.value.split("\n");
  let offset = 0;
  for (let line = 0; line < where.line && line < lines.length; line += 1) {
    offset += lines[line].length + 1;
  }
  return offset + where.character;
}

/// Which problem is open, so that an answer that arrives late for one
/// the reader has moved on from is dropped rather than shown.
let opening: Diagnostic | undefined;

/// Put away whatever a problem is offering.
function close(): void {
  opening = undefined;
  for (const offers of problems.querySelectorAll(".offers")) {
    offers.replaceChildren();
  }
}

/// What the server offers about a problem, under it.
async function offer(one: Diagnostic, into: HTMLUListElement): Promise<void> {
  if (!client) {
    return;
  }
  let fixes: Fix[] = [];
  try {
    fixes = await client.fixes(URI, one.range);
  } catch {
    // the server is the only one who knows, and it did not say
  }
  if (opening !== one) {
    return;
  }
  into.replaceChildren();
  if (fixes.length === 0) {
    const none = document.createElement("li");
    none.className = "nothing-near";
    none.textContent = "nothing near it";
    into.append(none);
    return;
  }
  for (const fix of fixes) {
    const offered = document.createElement("li");
    const apply = document.createElement("button");
    apply.className = "fix";
    apply.textContent = fix.title;
    apply.addEventListener("click", () => {
      replace(offsetOf(fix.range.start), offsetOf(fix.range.end), fix.text);
    });
    offered.append(apply);
    into.append(offered);
  }
}

/// The server's diagnostics, under the model they are about.
///
/// Clicking one puts the cursor where it is written and asks what the
/// name might have meant -- which is a question worth a walk over every
/// declared name, and so is asked on the click rather than published
/// with the problem.
function report(said: Diagnostic[]): void {
  problems.replaceChildren();
  opening = undefined;
  const errors = said.filter((one) => (one.severity ?? 1) === 1).length;
  if (said.length === 0) {
    say("No problems");
  } else {
    say(
      `${said.length} problem${said.length === 1 ? "" : "s"}` +
        (errors && errors !== said.length ? ` (${errors} error${errors === 1 ? "" : "s"})` : ""),
      errors > 0
    );
  }
  for (const one of said) {
    const item = document.createElement("li");
    item.className = (one.severity ?? 1) === 1 ? "error" : "warning";

    const where = document.createElement("span");
    where.className = "where";
    where.textContent = `${one.range.start.line + 1}:${one.range.start.character + 1}`;
    const message = document.createElement("span");
    message.textContent = one.message;
    const row = document.createElement("button");
    row.className = "problem";
    row.append(where, message);

    const offers = document.createElement("ul");
    offers.className = "offers";
    row.addEventListener("click", () => {
      const at = offsetOf(one.range.start);
      source.focus();
      source.setSelectionRange(at, at);
      const had = opening;
      close();
      if (had === one) {
        // clicked again: put it away and leave it away
        return;
      }
      opening = one;
      const waiting = document.createElement("li");
      waiting.className = "nothing-near";
      waiting.textContent = "\u2026";
      offers.append(waiting);
      void offer(one, offers);
    });

    item.append(row, offers);
    problems.append(item);
  }
}

// --------------------------------------------------------- what is saved

/// The drawing as its own file, with the theme it is being read in
/// written into it.
///
/// The SVG carries a palette for either theme and chooses with a media
/// query. Saved and opened again it is a document of its own, and which
/// way that query goes there is the viewer's business -- so the copy
/// says outright what this window settled on. What the query holds is
/// the dark half of the stylesheet: said again unqualified it is what
/// any viewer reads, dropped it leaves the light half standing.
function pinned(drawn: SVGSVGElement): string {
  const dark = window.matchMedia?.("(prefers-color-scheme: dark)").matches ?? false;
  const copy = drawn.cloneNode(true) as SVGSVGElement;
  let pinning = "";
  for (const sheet of Array.from(copy.querySelectorAll("style"))) {
    const query = (sheet.textContent ?? "").match(
      /@media[^{]*prefers-color-scheme[^{]*\{([\s\S]*)\}\s*$/
    );
    if (!query || query.index === undefined) {
      continue;
    }
    sheet.textContent = (sheet.textContent ?? "").slice(0, query.index);
    if (dark) {
      pinning += query[1];
    }
  }
  if (pinning !== "") {
    const pin = document.createElementNS("http://www.w3.org/2000/svg", "style");
    pin.textContent = pinning;
    copy.append(pin);
  }
  return new XMLSerializer().serializeToString(copy);
}

function save(name: string, blob: Blob): void {
  const link = document.createElement("a");
  link.href = URL.createObjectURL(blob);
  link.download = name;
  link.click();
  setTimeout(() => URL.revokeObjectURL(link.href), 10_000);
}

/// What a file can be saved as: the model as it was typed, and the
/// drawing in either of the two forms it is worth keeping.
type Kind = "sysml" | "svg" | "png";

/// What each is made of. The keys are the extensions a name may end in,
/// which is what [`kindOf`] reads them as.
const TYPES: Record<Kind, string> = {
  sysml: "text/plain;charset=utf-8",
  svg: "image/svg+xml",
  png: "image/png",
};

/// The drawing on the page, if there is one to save.
const drawn = (): SVGSVGElement | null => canvas.querySelector("svg");

/// What the file would hold.
async function contents(kind: Kind): Promise<Blob> {
  if (kind === "sysml") {
    return new Blob([source.value], { type: TYPES.sysml });
  }
  const svg = drawn();
  if (!svg) {
    throw new Error("there is no drawing to save");
  }
  return kind === "png" ? await rasterise(svg) : new Blob([pinned(svg)], { type: TYPES.svg });
}

/// What the file is called before anybody renames it.
function suggested(): string {
  return (chooseView.value === "internal" && element.value) || "model";
}

/// Which of the three a file name asks for, `undefined` for a name that
/// asks for none of them.
function kindOf(name: string): Kind | undefined {
  const at = name.lastIndexOf(".");
  const suffix = at === -1 ? "" : name.slice(at + 1).toLowerCase();
  return suffix in TYPES ? (suffix as Kind) : undefined;
}

/// A file name without the extension it is being saved as.
///
/// Only one of the three comes off. A model called `wheel.v2` is not a
/// `wheel` being saved as a `v2`, so renaming it leaves `wheel.v2` and
/// puts the extension after it.
function stemOf(name: string): string {
  return kindOf(name) ? name.slice(0, name.lastIndexOf(".")) : name;
}

/// Ask what to call it.
///
/// The page's own dialog, in every browser, and deliberately not
/// `showSaveFilePicker`.
///
/// That one can choose the folder as well, and what it does with the
/// name is the browser's: all it hands back is a name, never which type
/// the reader picked, and on at least one platform it ignores that
/// choice and puts the first type's extension onto a name that already
/// shows one -- `model.sysml.sysml` for a drawing asked for as SVG. A
/// page cannot reach into that dialog and correct it, so it does not
/// reach for it at all. Where the file lands is then the browser's
/// download setting, which can be told to ask.
function saveAs(): void {
  ask(drawn() ? ["sysml", "svg", "png"] : ["sysml"]);
}

/// The name goes to an ordinary download, so where it lands is the
/// browser's business rather than this page's.
function ask(kinds: Kind[]): void {
  for (const option of saveKind.options) {
    option.disabled = !kinds.includes(option.value as Kind);
  }
  saveKind.value = "sysml";
  // The box holds a name and the menu beside it holds the extension, so
  // the two read as one file name and neither can contradict the other.
  // An extension in the box instead would double the moment a reader
  // typed the whole name they wanted over it -- `model.sysml.sysml` --
  // and would leave the menu nothing to decide.
  saveName.value = suggested();
  saving.hidden = false;
  saveName.focus();
  saveName.select();
}

/// A name typed with an extension on it still says which type it is, so
/// the menu follows it. What is saved takes the extension off again and
/// puts the menu's on, so the two can never disagree.
function retype(): void {
  const kind = kindOf(saveName.value.trim());
  const option = kind && saveKind.querySelector<HTMLOptionElement>(`option[value="${kind}"]`);
  if (option && !option.disabled) {
    saveKind.value = kind;
  }
}

function unask(): void {
  saving.hidden = true;
}

/// The drawing as a PNG, at twice its size so the text stays readable.
/// The page's own background is painted behind it: the drawing has none,
/// and a dark theme's pale lines would be saved onto nothing at all.
function rasterise(drawn: SVGSVGElement): Promise<Blob> {
  return new Promise((resolve, reject) => {
    const image = new Image();
    image.onload = () => {
      const ratio = 2;
      const page = document.createElement("canvas");
      page.width = Math.max(1, Math.round(natural.width * ratio));
      page.height = Math.max(1, Math.round(natural.height * ratio));
      const pen = page.getContext("2d");
      if (!pen) {
        reject(new Error("this browser would not give the page a canvas"));
        return;
      }
      pen.scale(ratio, ratio);
      pen.fillStyle = getComputedStyle(document.body).backgroundColor || "#ffffff";
      pen.fillRect(0, 0, natural.width, natural.height);
      pen.drawImage(image, 0, 0, natural.width, natural.height);
      page.toBlob((png) =>
        png ? resolve(png) : reject(new Error("the drawing would not rasterise"))
      );
    };
    image.onerror = () => reject(new Error("the drawing did not load"));
    image.src = `data:image/svg+xml;charset=utf-8,${encodeURIComponent(pinned(drawn))}`;
  });
}

// ------------------------------------------------------------- the page

/// Which of the toolbar's inputs the view being drawn has a use for:
/// only an internal view is of one named element.
function controls(): void {
  element.hidden = chooseView.value !== "internal";
}

function load(index: number): void {
  const example = EXAMPLES[index];
  source.value = example.source;
  chooseView.value = example.view;
  element.value = example.element ?? "";
  chosen = false;
  controls();
  edited();
}

function wire(): void {
  for (const [index, example] of EXAMPLES.entries()) {
    const option = document.createElement("option");
    option.value = String(index);
    option.textContent = example.name;
    chooseExample.append(option);
  }
  chooseExample.addEventListener("change", () => {
    load(Number(chooseExample.value));
    // back to the menu's own label, so it reads as a menu rather than
    // as a claim about what is in the editor now
    chooseExample.selectedIndex = 0;
  });

  source.addEventListener("input", edited);
  source.addEventListener("scroll", follow);
  chooseView.addEventListener("change", () => {
    controls();
    chosen = false;
    schedule();
  });
  element.addEventListener("input", schedule);

  // A textarea that moves focus on Tab cannot be typed SysML into.
  // Escape is what lets a reader using the keyboard out of it again,
  // and is handled below, for the window: it has something to close
  // before it has a box to leave.
  source.addEventListener("keydown", (event) => {
    if (event.key !== "Tab" || event.shiftKey || event.ctrlKey || event.metaKey) {
      return;
    }
    event.preventDefault();
    replace(source.selectionStart, source.selectionEnd, "    ");
  });

  // Escape puts away whatever is open, innermost first: the offers under
  // a problem, then the editor itself. Listened for on the window rather
  // than on the textarea, since what it closes may be what has the focus
  // -- a reader who reached an offer with the keyboard is standing on a
  // button inside it.
  window.addEventListener("keydown", (event) => {
    if (event.key !== "Escape") {
      return;
    }
    if (!saving.hidden) {
      unask();
      event.preventDefault();
      return;
    }
    if (opening) {
      close();
      source.focus();
      event.preventDefault();
      return;
    }
    if (document.activeElement === source) {
      source.blur();
    }
  });

  need("in").addEventListener("click", () => zoomBy(STEP));
  need("out").addEventListener("click", () => zoomBy(1 / STEP));
  need("fit").addEventListener("click", fit);
  need("actual").addEventListener("click", () => {
    scale = 1;
    chosen = true;
    apply();
  });

  // ctrl (or command) and the wheel zooms towards the pointer; the
  // wheel alone keeps scrolling, which is what a long diagram wants
  pane.addEventListener(
    "wheel",
    (event) => {
      if (!event.ctrlKey && !event.metaKey) {
        return;
      }
      event.preventDefault();
      zoomAt(scale * Math.pow(1.0015, -event.deltaY), event.clientX, event.clientY);
    },
    { passive: false }
  );

  let panning: { x: number; y: number; left: number; top: number } | undefined;
  pane.addEventListener("pointerdown", (event) => {
    if (event.button !== 0) {
      return;
    }
    panning = { x: event.clientX, y: event.clientY, left: pane.scrollLeft, top: pane.scrollTop };
    pane.setPointerCapture(event.pointerId);
    pane.classList.add("panning");
  });
  pane.addEventListener("pointermove", (event) => {
    if (!panning) {
      return;
    }
    pane.scrollLeft = panning.left - (event.clientX - panning.x);
    pane.scrollTop = panning.top - (event.clientY - panning.y);
  });
  const rest = () => {
    panning = undefined;
    pane.classList.remove("panning");
  };
  pane.addEventListener("pointerup", rest);
  pane.addEventListener("pointercancel", rest);

  // The handle between the panes. `setPointerCapture` is what keeps the
  // drag with it once the pointer has run ahead of it, which it does the
  // moment a pane stops being able to give way.
  split.addEventListener("pointerdown", (event) => {
    split.setPointerCapture(event.pointerId);
    split.classList.add("dragging");
    event.preventDefault();
  });
  split.addEventListener("pointermove", (event) => {
    if (split.hasPointerCapture(event.pointerId)) {
      divide(shareAt(event.clientX));
    }
  });
  const dropped = (event: PointerEvent) => {
    split.releasePointerCapture?.(event.pointerId);
    split.classList.remove("dragging");
  };
  split.addEventListener("pointerup", dropped);
  split.addEventListener("pointercancel", dropped);
  // and the same handle from the keyboard, since it is a control
  split.addEventListener("keydown", (event) => {
    const step = event.key === "ArrowLeft" ? -4 : event.key === "ArrowRight" ? 4 : 0;
    if (step === 0) {
      return;
    }
    event.preventDefault();
    divide(share + step);
  });
  split.addEventListener("dblclick", () => divide(50));

  const link = need<HTMLButtonElement>("link");
  const LINK = link.textContent;
  link.addEventListener("click", async () => {
    remember();
    try {
      await navigator.clipboard.writeText(location.href);
      link.textContent = "Copied";
    } catch {
      // a browser that will not hand over the clipboard: the address bar
      // already says the same thing, so say where to find it
      link.textContent = "It is in the address bar";
    }
    setTimeout(() => {
      link.textContent = LINK;
    }, 1600);
  });

  need("save").addEventListener("click", saveAs);
  need("save-cancel").addEventListener("click", unask);
  saveName.addEventListener("input", retype);
  saving.addEventListener("click", (event) => {
    // the sheet's own backdrop, which is the rest of the page
    if (event.target === saving) {
      unask();
    }
  });
  need("save-as").addEventListener("submit", async (event) => {
    event.preventDefault();
    const kind = saveKind.value as Kind;
    // whatever extension was typed comes off and the menu's goes on, so
    // what is saved is what the two controls say together
    const name = stemOf(saveName.value.trim()) || suggested();
    unask();
    try {
      save(`${name}.${kind}`, await contents(kind));
    } catch (why) {
      say(`${why instanceof Error ? why.message : why}`, true);
    }
  });
}

async function start(): Promise<void> {
  wire();
  window.addEventListener("resize", () => {
    settle();
    apply();
  });
  const shared = opened();
  if (shared) {
    source.value = shared.source;
    chooseView.value = shared.view;
    element.value = shared.element;
    controls();
    number();
    repaint();
  } else {
    load(0);
  }

  // Colouring is the editor's own business and does not wait on the
  // language server: the grammar is seven kilobytes and the engine's
  // module half a megabyte, against six for the server.
  const colouring = highlighter("sysml.tmLanguage.json", "onig.wasm")
    .then((ready) => {
      paint = ready;
      repaint();
      follow();
    })
    .catch(() => {
      // an editor with no colour is still an editor
      say("the grammar did not load; the model is shown uncoloured");
    });

  // The fetch is a couple of megabytes over the wire, and the handshake
  // reads the standard library -- 94 files, resolved before the server
  // answers -- so this is the one slow moment in a session.
  const loading = "Loading the language server…";
  say(loading);
  canvas.replaceChildren(nothing(loading));
  try {
    client = await Client.start("sysml-lsp.wasm", "server.worker.js");
  } catch (why) {
    const said = why instanceof Error ? why.message : String(why);
    say(said, true);
    canvas.replaceChildren(nothing(said));
    return;
  }
  client.onDiagnostics = (uri, said) => {
    if (uri === URI) {
      report(said);
    }
  };
  client.onLog = (said) => say(said, true);

  sent = source.value;
  client.open(URI, sent);
  await draw();
  // so that a test, and anyone watching the tab, has one moment when
  // everything the page loads is loaded
  await colouring;
}

void start();
