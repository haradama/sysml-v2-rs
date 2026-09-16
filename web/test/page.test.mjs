// The playground, driven the way a reader drives it.
//
// The built page is loaded into a DOM and given a `Worker` that runs the
// built worker, which runs the `.wasm` that will be published. So what
// is checked is the whole crossing -- a keystroke in the textarea, the
// Language Server Protocol, the parser, name resolution, the layout and
// the SVG -- rather than a description of it written out again here.
//
// jsdom lays nothing out, so the pane is told its own size; the drawing
// brings its own, which is what a zoom is a factor of anyway.

import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import test from "node:test";
import { fileURLToPath } from "node:url";
import vm from "node:vm";
import { JSDOM } from "jsdom";

const PAGE = new URL("../dist/index.html", import.meta.url);
const APP = new URL("../dist/app.js", import.meta.url);
const WORKER = new URL("../dist/server.worker.js", import.meta.url);
const WASM = new URL("../dist/sysml-lsp.wasm", import.meta.url);

const built = [PAGE, APP, WORKER, WASM].every((at) => existsSync(fileURLToPath(at)));
const unbuilt = {
  skip: built ? false : "nothing in dist/ -- run `npm run build`",
};

const read = (at) => readFileSync(fileURLToPath(at), "utf8");

/// The worker the page opens, running the real one. A `Worker` is
/// another realm with its own globals and a `postMessage` that goes the
/// other way, which is what this is: `node:vm` for the realm, and the
/// two `postMessage`s wired to each other.
function workerAgainst(window) {
  return class Worker {
    constructor() {
      const sandbox = {
        postMessage: (message) => {
          // asynchronous, as a real one is: the page must not depend on
          // an answer arriving inside the call that asked for it
          setTimeout(() => this.onmessage?.({ data: message }), 0);
        },
        TextEncoder,
        TextDecoder,
        WebAssembly,
        JSON,
        Uint8Array,
        setTimeout,
        clearTimeout,
        console,
        onmessage: null,
      };
      vm.createContext(sandbox);
      vm.runInContext(read(WORKER), sandbox);
      this.sandbox = sandbox;
      this.onmessage = null;
      window.__worker = this;
    }

    postMessage(message) {
      setTimeout(() => this.sandbox.onmessage({ data: message }), 0);
    }
  };
}

/// A loaded page, with the handles a test needs to drive it.
function open(hash = "", over = {}) {
  const dom = new JSDOM(read(PAGE), {
    url: `https://example.invalid/${hash}`,
    runScripts: "outside-only",
    pretendToBeVisual: true,
  });
  const { window } = dom;
  const { document } = window;

  window.TextEncoder = TextEncoder;
  window.TextDecoder = TextDecoder;
  window.WebAssembly = WebAssembly;
  window.Worker = workerAgainst(window);
  // Everything the page fetches, served off disk. The language server
  // is handed a fresh copy each time, since the page transfers it away.
  window.fetch = over.fetch ?? (async (what) => {
    const at = new URL(String(what), "https://example.invalid/").pathname.slice(1);
    const file = new URL(`../dist/${at}`, import.meta.url);
    return {
      ok: true,
      status: 200,
      arrayBuffer: async () => new Uint8Array(readFileSync(fileURLToPath(file))).buffer,
      text: async () => readFileSync(fileURLToPath(file), "utf8"),
    };
  });

  const pane = document.getElementById("pane");
  Object.defineProperty(pane, "clientWidth", { value: 800 });
  Object.defineProperty(pane, "clientHeight", { value: 600 });

  window.eval(read(APP));

  const at = (id) => document.getElementById(id);
  return {
    window,
    document,
    at,
    /// Type into the model, the way the page hears it.
    type(said) {
      const source = at("source");
      source.value = said;
      source.dispatchEvent(new window.Event("input"));
    },
    /// Wait for the page to say something a test is waiting for. The
    /// module is compiled and the standard library is read -- 94 files
    /// -- while the first of these waits.
    async until(wanted, why) {
      for (let tick = 0; tick < 1200; tick += 1) {
        if (wanted()) {
          return;
        }
        await new Promise((resume) => setTimeout(resume, 25));
      }
      assert.fail(`${why}; the page says "${at("state").textContent}"`);
    },
  };
}

/// The SVG now on the page, if there is one.
const drawing = (page) => page.at("canvas").querySelector("svg");

/// What the editor's painted layer makes of `text`, as `[text, class]`
/// for everything it coloured.
function coloured(page) {
  return [...page.at("painted").querySelectorAll("span")].map((span) => [
    span.textContent,
    span.className,
  ]);
}

/// Wait until the grammar has loaded and the editor is being coloured.
const painting = (page) =>
  page.until(
    () => page.at("painted").querySelector("span"),
    "the editor was never coloured"
  );

test("the page draws the model it opens with", unbuilt, async () => {
  const page = open();
  await page.until(() => drawing(page), "nothing was drawn");
  const svg = drawing(page);
  assert.match(svg.outerHTML, /Engine/, "the first example's definitions");
  assert.match(svg.outerHTML, /PowerSource/);
  // drawn at the size the renderer says, which is what a zoom scales
  assert.ok(Number(svg.getAttribute("width")) > 0);
  assert.equal(page.at("state").textContent, "No problems");
});

test("a name that resolves to nothing is reported where it is written", unbuilt, async () => {
  const page = open();
  await page.until(() => drawing(page), "nothing was drawn");

  page.type("part def Car {\n    part w : Wheeel;\n}\n");
  await page.until(
    () => page.at("problems").children.length > 0,
    "the typo was not reported"
  );
  const [first] = page.at("problems").children;
  assert.match(first.textContent, /Wheeel/);
  // line 2, character 5, as a reader counts them
  assert.match(first.querySelector(".where").textContent, /^2:/);
  assert.match(page.at("state").textContent, /1 problem/);
});

test("a name from the standard library resolves, with no library to load", unbuilt, async () => {
  const page = open();
  await page.until(() => drawing(page), "nothing was drawn");

  // `ISQ::MassValue` is the library's, and the library is inside the
  // module: there is nowhere else this page could read it from
  page.type("part def Car {\n    attribute mass : ISQ::MassValue;\n}\n");
  await page.until(
    () => drawing(page)?.outerHTML.includes("Car"),
    "the new model was not drawn"
  );
  assert.equal(page.at("problems").children.length, 0);
});

test("the view menu picks what is drawn", unbuilt, async () => {
  const page = open();
  await page.until(() => drawing(page), "nothing was drawn");

  const view = page.at("view");
  view.value = "browser";
  view.dispatchEvent(new page.window.Event("change"));
  await page.until(
    () => drawing(page)?.outerHTML.includes("VehicleDefinitions"),
    "the tree was not drawn"
  );
  // the element box belongs to the internal view and to no other
  assert.equal(page.at("element").hidden, true);

  view.value = "internal";
  view.dispatchEvent(new page.window.Event("change"));
  assert.equal(page.at("element").hidden, false);
});

test("an internal view with nothing named says so", unbuilt, async () => {
  const page = open();
  await page.until(() => drawing(page), "nothing was drawn");

  const view = page.at("view");
  view.value = "internal";
  view.dispatchEvent(new page.window.Event("change"));
  await page.until(
    () => page.at("canvas").textContent.includes("Name the element"),
    "the empty element box was not explained"
  );

  const element = page.at("element");
  element.value = "Nowhere";
  element.dispatchEvent(new page.window.Event("input"));
  await page.until(
    () => page.at("canvas").textContent.includes("Nowhere"),
    "a name that is nowhere was not reported"
  );
  assert.equal(drawing(page), null, "and nothing was drawn for it");
});

test("the address bar carries the model, and a link opens it again", unbuilt, async () => {
  const page = open();
  await page.until(() => drawing(page), "nothing was drawn");

  // the example that opened the page is already in the address bar, so
  // what this waits for is the address bar catching up with the typing
  const was = page.window.location.hash;
  page.type("part def Widget;\n");
  await page.until(
    () => page.window.location.hash !== was,
    "the typing did not reach the address bar"
  );
  const shared = page.window.location.hash;
  assert.ok(!shared.includes("part def Widget"), "packed, not written out");

  // the same link, opened cold
  const opened = open(shared);
  assert.equal(opened.at("source").value, "part def Widget;\n");
  await opened.until(
    () => drawing(opened)?.outerHTML.includes("Widget"),
    "the shared model was not drawn"
  );
});

test("the editor is coloured by the grammar the extension ships", unbuilt, async () => {
  const page = open();
  await painting(page);

  page.type(
    "package P {\n" +
      '    doc /* what it is for */\n' +
      "    // an aside\n" +
      "    #approved\n" +
      "    part def Wheel;\n" +
      "    part w : Wheel[4] = \"four\";\n" +
      "}\n"
  );
  await page.until(
    () => coloured(page).some(([text]) => text === "#approved"),
    "the model that was typed was not coloured"
  );

  // the token the grammar hands back may carry more than the word, so
  // this looks the way `editors/vscode/test/grammar.test.mjs` looks
  const paint = (text) => coloured(page).find(([had]) => had.includes(text))?.[1];
  assert.equal(paint("package"), "t-keyword");
  assert.equal(paint("part"), "t-keyword");
  assert.equal(paint("def"), "t-keyword");
  assert.equal(paint("Wheel"), "t-type");
  assert.equal(paint("#approved"), "t-tag");
  assert.equal(paint("4"), "t-number");
  // a `doc` is the model's own prose and an aside is not; the grammar
  // is careful to tell them apart, and so is the page
  assert.equal(paint("what it is for"), "t-doc");
  assert.equal(paint("// an aside"), "t-note");

  // The painted layer carries the model and one newline more. `<pre>`
  // drops a single trailing newline when it renders and a textarea does
  // not, so that extra one is what makes the two layers the same height
  // -- which is what keeps a colour over the text it belongs to.
  assert.equal(page.at("painted").textContent, `${page.at("source").value}\n`);
});

/// The painted layer is written with `innerHTML`, so a model is markup
/// until it is escaped. Nothing about a model is trusted: one arrives in
/// the address bar, from whoever sent the link.
test("a model cannot write markup into the page", unbuilt, async () => {
  const page = open();
  await painting(page);

  page.type('part def A { doc /* <img src=x onerror="boom"> */ }\n');
  await page.until(
    () => page.at("painted").textContent.includes("<img"),
    "the model was not coloured"
  );
  const layer = page.at("painted");
  assert.equal(layer.querySelectorAll("img").length, 0, "no element was made");
  assert.equal(layer.querySelectorAll("[onerror]").length, 0);
  // it is there, as the text it is
  assert.ok(layer.textContent.includes('<img src=x onerror="boom">'));
});

/// A grammar that never arrives must leave an editor somebody can read,
/// not an empty box: the text only goes transparent once there is a
/// coloured copy of it underneath.
test("without the grammar the editor still shows its text", unbuilt, async () => {
  const page = open("", {
    fetch: async (what) => {
      if (String(what).includes("onig")) {
        return { ok: false, status: 404 };
      }
      const at = new URL(String(what), "https://example.invalid/").pathname.slice(1);
      const file = new URL(`../dist/${at}`, import.meta.url);
      return {
        ok: true,
        status: 200,
        arrayBuffer: async () => new Uint8Array(readFileSync(fileURLToPath(file))).buffer,
        text: async () => readFileSync(fileURLToPath(file), "utf8"),
      };
    },
  });
  await page.until(() => drawing(page), "nothing was drawn");
  assert.equal(page.at("painted").querySelectorAll("span").length, 0);
  assert.ok(!page.at("source").parentElement.classList.contains("painted"));
  assert.match(page.at("source").value, /VehicleDefinitions/);
});

test("every line is numbered, and the numbers keep up", unbuilt, async () => {
  const page = open();
  await page.until(() => drawing(page), "nothing was drawn");

  const numbers = () => page.at("gutter").textContent.trim().split("\n");
  const lines = () => page.at("source").value.split("\n");
  assert.deepEqual(numbers().length, lines().length);
  assert.equal(numbers()[0], "1");

  page.type("part def A;\npart def B;\npart def C;\n");
  await page.until(() => numbers().length === 4, "the numbers did not keep up");
  assert.deepEqual(numbers(), ["1", "2", "3", "4"]);

  // the gutter carries one newline more than there are numbers, the way
  // the painted layer does, so the three layers are the same height
  assert.equal(page.at("gutter").textContent, "1\n2\n3\n4\n");
});

/// What a name that resolved to nothing might have meant. The server
/// answers it, so this is the whole way round: the click, the request,
/// the edit, and the model it leaves behind.
test("clicking a problem offers what the name might have meant", unbuilt, async () => {
  const page = open();
  await page.until(() => drawing(page), "nothing was drawn");

  // `Garage` rather than `Parts`, which the standard library has already
  // taken and would leave a collision behind after the typo is fixed
  page.type("package Garage {\n    part def Wheel;\n}\npackage P {\n    part a : Wheeel;\n}\n");
  await page.until(
    () => [...page.at("problems").querySelectorAll(".problem")].some((one) =>
      one.textContent.includes("Wheeel")
    ),
    "the typo was not reported"
  );

  const row = [...page.at("problems").querySelectorAll(".problem")].find((one) =>
    one.textContent.includes("Wheeel")
  );
  // nothing is offered until it is asked for
  assert.equal(page.at("problems").querySelectorAll(".fix").length, 0);
  row.click();
  await page.until(
    () => page.at("problems").querySelector(".fix"),
    "nothing was offered for the typo"
  );

  const fix = page.at("problems").querySelector(".fix");
  assert.match(fix.textContent, /did you mean `Garage::Wheel`\?/);

  // and it is an edit, not a note
  fix.click();
  await page.until(
    () => page.at("source").value.includes("Garage::Wheel"),
    "the fix was not applied"
  );
  assert.ok(!page.at("source").value.includes("Wheeel"), page.at("source").value);
  await page.until(
    () => page.at("problems").querySelectorAll(".problem").length === 0,
    "the model still has something wrong with it"
  );
});

/// The two mistakes are not the same mistake, and the offer says which:
/// a name nothing declares was mistyped, a name something declares was
/// never in scope here.
test("a name that is declared elsewhere is offered as that, not as a typo", unbuilt, async () => {
  const page = open();
  await page.until(() => drawing(page), "nothing was drawn");

  page.type("package Where {\n    part def Wheel;\n}\npackage P {\n    part b : Wheel;\n}\n");
  await page.until(
    () => page.at("problems").querySelector(".problem"),
    "nothing was reported"
  );
  page.at("problems").querySelector(".problem").click();
  await page.until(
    () => page.at("problems").querySelector(".fix"),
    "nothing was offered"
  );
  assert.match(
    page.at("problems").querySelector(".fix").textContent,
    /`Where::Wheel`, declared elsewhere/
  );
});

test("escape puts the offers away and hands the editor back", unbuilt, async () => {
  const page = open();
  await page.until(() => drawing(page), "nothing was drawn");

  page.type("package Garage {\n    part def Wheel;\n}\npackage P {\n    part a : Wheeel;\n}\n");
  await page.until(
    () => page.at("problems").querySelector(".problem"),
    "the typo was not reported"
  );
  page.at("problems").querySelector(".problem").click();
  await page.until(() => page.at("problems").querySelector(".fix"), "nothing was offered");

  // from a button inside the offers, which is where a reader who got
  // there with the keyboard is standing
  page.at("problems").querySelector(".fix").focus();
  page.window.dispatchEvent(
    new page.window.KeyboardEvent("keydown", { key: "Escape", bubbles: true })
  );
  assert.equal(page.at("problems").querySelectorAll(".fix").length, 0, "the offers stayed");
  assert.equal(page.document.activeElement, page.at("source"), "the editor did not get it back");

  // and with nothing open it is what leaves the editor, so a reader
  // using the keyboard is not shut inside it
  page.at("source").focus();
  page.window.dispatchEvent(
    new page.window.KeyboardEvent("keydown", { key: "Escape", bubbles: true })
  );
  assert.notEqual(page.document.activeElement, page.at("source"));
});

/// A textarea keeps its own undo history and nothing written into
/// `value` joins it, so an edit the page makes has to go through
/// `execCommand` or a reader cannot take it back. jsdom has no undo to
/// check, but it can check that the edit went the way that has one.
test("an edit the page makes is one the editor can take back", unbuilt, async () => {
  const page = open();
  await page.until(() => drawing(page), "nothing was drawn");

  const asked = [];
  page.document.execCommand = (command, _ui, text) => {
    asked.push(command);
    const box = page.at("source");
    const { selectionStart: from, selectionEnd: to } = box;
    box.value = box.value.slice(0, from) + text + box.value.slice(to);
    box.setSelectionRange(from + text.length, from + text.length);
    return true;
  };

  page.type("package Garage {\n    part def Wheel;\n}\npackage P {\n    part a : Wheeel;\n}\n");
  await page.until(
    () => page.at("problems").querySelector(".problem"),
    "the typo was not reported"
  );
  page.at("problems").querySelector(".problem").click();
  await page.until(() => page.at("problems").querySelector(".fix"), "nothing was offered");
  page.at("problems").querySelector(".fix").click();

  assert.deepEqual(asked, ["insertText"], "the fix did not go through the undoable edit");
  assert.ok(page.at("source").value.includes("Garage::Wheel"), page.at("source").value);
});

test("the handle divides the window and keeps both panes usable", unbuilt, async () => {
  const page = open();
  await page.until(() => drawing(page), "nothing was drawn");

  const split = page.at("split");
  const left = () => page.at("source").closest("main").style.getPropertyValue("--left");
  const press = (key) =>
    split.dispatchEvent(new page.window.KeyboardEvent("keydown", { key, bubbles: true }));

  press("ArrowRight");
  assert.equal(left(), "54%");
  assert.equal(split.getAttribute("aria-valuenow"), "54");
  press("ArrowLeft");
  press("ArrowLeft");
  assert.equal(left(), "46%");

  // neither pane may be shut away altogether
  for (let push = 0; push < 40; push += 1) {
    press("ArrowLeft");
  }
  assert.equal(left(), "12%");
  for (let push = 0; push < 60; push += 1) {
    press("ArrowRight");
  }
  assert.equal(left(), "88%");

  // and back to the middle, which is what a double click is for
  split.dispatchEvent(new page.window.MouseEvent("dblclick", { bubbles: true }));
  assert.equal(left(), "50%");
});

/// One button, one dialog, and it is this page's rather than the
/// browser's -- so what comes out is what the two controls say, in every
/// browser. `showSaveFilePicker` would choose the folder as well, and on
/// at least one platform it ignores the type the reader picks and hands
/// back `model.sysml.sysml` for a drawing asked for as SVG.
test("saving asks for the name, and the menu decides the extension", unbuilt, async () => {
  const page = open();
  await page.until(() => drawing(page), "nothing was drawn");

  const saved = [];
  page.window.URL.createObjectURL = () => "blob:stub";
  page.window.URL.revokeObjectURL = () => {};
  page.document.createElement = new Proxy(page.document.createElement, {
    apply(make, self, args) {
      const made = Reflect.apply(make, self, args);
      if (args[0] === "a") {
        made.click = () => saved.push(made.download);
      }
      return made;
    },
  });

  assert.ok(page.at("saving").hidden, "the sheet was open before it was asked for");
  page.at("save").click();
  assert.ok(!page.at("saving").hidden, "the sheet did not open");
  // a name, with the extension in the menu beside it rather than in it
  assert.equal(page.at("save-name").value, "model");
  assert.deepEqual(
    [page.at("save-name").selectionStart, page.at("save-name").selectionEnd],
    [0, "model".length]
  );

  const named = (text) => {
    page.at("save-name").value = text;
    page.at("save-name").dispatchEvent(new page.window.Event("input", { bubbles: true }));
  };
  const typed = (kind) => {
    page.at("save-kind").value = kind;
    page.at("save-kind").dispatchEvent(new page.window.Event("change", { bubbles: true }));
  };
  const submit = async () => {
    const had = saved.length;
    page.at("save-as").dispatchEvent(
      new page.window.Event("submit", { bubbles: true, cancelable: true })
    );
    await page.until(() => saved.length > had, "nothing was saved");
    return saved[saved.length - 1];
  };

  // a name typed with an extension on it moves the menu to match
  named("elsewhere.svg");
  assert.equal(page.at("save-kind").value, "svg");

  // and the menu is what decides, so the extension cannot double and
  // cannot disagree with what was chosen
  named("model.sysml");
  typed("svg");
  assert.equal(await submit(), "model.svg");

  page.at("save").click();
  named("model.sysml");
  typed("sysml");
  assert.equal(await submit(), "model.sysml");

  // a name whose own last dot is not one of the three keeps it.
  // `.svg` rather than `.png`: rasterising wants a canvas, and jsdom has
  // none -- what a PNG comes out as is checked in a real browser
  page.at("save").click();
  named("wheel.v2");
  typed("svg");
  assert.equal(await submit(), "wheel.v2.svg");

  page.at("save").click();
  named("my-diagram");
  typed("svg");
  assert.equal(await submit(), "my-diagram.svg");
  assert.ok(page.at("saving").hidden, "the sheet stayed open");

  // and escape puts it away
  page.at("save").click();
  assert.ok(!page.at("saving").hidden);
  page.window.dispatchEvent(
    new page.window.KeyboardEvent("keydown", { key: "Escape", bubbles: true })
  );
  assert.ok(page.at("saving").hidden, "escape did not put the sheet away");
});

test("a link that arrived truncated falls back to the first example", unbuilt, async () => {
  const page = open("#view=definitions&m=!!!not-base64!!!");
  assert.match(page.at("source").value, /VehicleDefinitions/);
});
