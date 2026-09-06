// The preview page, driven the way a reader drives it.
//
// The page is loaded into a DOM and given the messages the extension
// posts, so what is checked here is the built page itself rather than a
// copy of it. jsdom lays nothing out, so the scroller is told its own
// size and the drawing brings its own, which is what a zoom is a factor
// of anyway.

import assert from "node:assert/strict";
import { createRequire } from "node:module";
import test from "node:test";
import { JSDOM } from "jsdom";

const { page } = createRequire(import.meta.url)("../out/page.js");

const DRAWING =
  '<svg xmlns="http://www.w3.org/2000/svg" width="800" height="600"><rect/></svg>';

/// A loaded page, with the handles a test needs to drive it.
function open() {
  const dom = new JSDOM(page(), {
    runScripts: "outside-only",
    pretendToBeVisual: true,
  });
  const { window } = dom;
  const posted = [];
  let kept = {};
  window.acquireVsCodeApi = () => ({
    postMessage: (message) => posted.push(message),
    setState: (state) => {
      kept = state;
    },
    getState: () => kept,
  });
  window.eval(
    window.document.querySelector("script").textContent
  );

  const document = window.document;
  const diagram = document.getElementById("diagram");
  Object.defineProperty(diagram, "clientWidth", { value: 400 });
  Object.defineProperty(diagram, "clientHeight", { value: 300 });
  diagram.getBoundingClientRect = () => ({
    left: 0,
    top: 0,
    width: 400,
    height: 300,
    right: 400,
    bottom: 300,
  });
  const scroll = { left: 0, top: 0 };
  Object.defineProperty(diagram, "scrollLeft", {
    get: () => scroll.left,
    set: (to) => {
      scroll.left = to;
    },
  });
  Object.defineProperty(diagram, "scrollTop", {
    get: () => scroll.top,
    set: (to) => {
      scroll.top = to;
    },
  });

  const at = (id) => document.getElementById(id);
  return {
    window,
    document,
    posted,
    scroll,
    at,
    state: () => kept,
    /// The last message the page posted, as a plain object: jsdom's realm
    /// has prototypes of its own, which a strict comparison would see.
    last: () => ({ ...posted.at(-1) }),
    zoom: () => Number(at("zoom").textContent.replace("%", "")),
    sizer: at("sizer"),
    canvas: at("canvas"),
    draw: (kind, body, view = "definitions", element = "") =>
      window.dispatchEvent(
        new window.MessageEvent("message", {
          data: { command: "draw", kind, body, view, element },
        })
      ),
    click: (id) =>
      at(id).dispatchEvent(new window.MouseEvent("click", { bubbles: true })),
    wheel: (options) =>
      diagram.dispatchEvent(
        new window.WheelEvent("wheel", {
          bubbles: true,
          cancelable: true,
          ...options,
        })
      ),
    mouse: (type, options, on = diagram) =>
      on.dispatchEvent(
        new window.MouseEvent(type, { bubbles: true, ...options })
      ),
    key: (key, on = document.body) =>
      on.dispatchEvent(
        new window.KeyboardEvent("keydown", { bubbles: true, key })
      ),
  };
}

test("the page asks for a drawing as soon as it is loaded", () => {
  const it = open();
  assert.equal(it.posted.length, 1);
  assert.deepEqual(it.last(), { command: "ready" });
});

test("a drawing is shown at the size it was drawn", () => {
  const it = open();
  it.draw("svg", DRAWING);
  assert.equal(it.zoom(), 100);
  assert.equal(it.sizer.style.width, "800px");
  assert.equal(it.sizer.style.height, "600px");
});

test("the buttons zoom in and out by a fifth at a time", () => {
  const it = open();
  it.draw("svg", DRAWING);
  it.click("in");
  assert.equal(it.zoom(), 120);
  assert.equal(it.sizer.style.width, `${800 * 1.2}px`);
  it.click("out");
  assert.equal(it.zoom(), 100);
});

test("fit takes the tighter of width and height, and 1:1 undoes it", () => {
  const it = open();
  it.draw("svg", DRAWING);
  it.click("fit");
  // 284 of 600 is tighter than 384 of 800
  assert.equal(it.zoom(), 47);
  it.click("reset");
  assert.equal(it.zoom(), 100);
  assert.equal(it.canvas.style.transform, "scale(1)");
});

test("a redraw keeps the zoom the reader chose, and a reload remembers it", () => {
  const it = open();
  it.draw("svg", DRAWING);
  it.click("in");
  it.draw("svg", DRAWING);
  assert.equal(it.zoom(), 120);
  assert.equal(Math.round(it.state().scale * 100), 120);
});

test("ctrl and the wheel zoom towards the pointer", () => {
  const it = open();
  it.draw("svg", DRAWING);
  it.wheel({ ctrlKey: true, deltaY: -100, clientX: 200, clientY: 150 });
  assert.ok(it.zoom() > 100, `zoomed to ${it.zoom()}%`);
  // what was under the pointer is still under it
  const scale = it.zoom() / 100;
  assert.equal(Math.round(it.scroll.left), Math.round(200 * scale - 200));
  assert.equal(Math.round(it.scroll.top), Math.round(150 * scale - 150));
});

test("the wheel by itself is left to scroll", () => {
  const it = open();
  it.draw("svg", DRAWING);
  it.wheel({ deltaY: -100, clientX: 200, clientY: 150 });
  assert.equal(it.zoom(), 100);
  assert.equal(it.scroll.left, 0);
});

test("dragging moves the drawing under the pointer", () => {
  const it = open();
  it.draw("svg", DRAWING);
  it.mouse("mousedown", { button: 0, clientX: 100, clientY: 100 });
  assert.ok(it.at("diagram").classList.contains("panning"));
  it.mouse("mousemove", { clientX: 70, clientY: 60 }, it.window);
  assert.deepEqual([it.scroll.left, it.scroll.top], [30, 40]);
  it.mouse("mouseup", {}, it.window);
  assert.ok(!it.at("diagram").classList.contains("panning"));
});

test("a drag with the other buttons is not a pan", () => {
  const it = open();
  it.draw("svg", DRAWING);
  it.mouse("mousedown", { button: 2, clientX: 100, clientY: 100 });
  it.mouse("mousemove", { clientX: 70, clientY: 60 }, it.window);
  assert.deepEqual([it.scroll.left, it.scroll.top], [0, 0]);
});

test("the keyboard does what the buttons do, unless a name is being typed", () => {
  const it = open();
  it.draw("svg", DRAWING);
  it.key("+");
  assert.equal(it.zoom(), 120);
  it.key("-");
  assert.equal(it.zoom(), 100);
  it.key("=");
  assert.equal(it.zoom(), 120);
  it.key("0");
  assert.equal(it.zoom(), 100);
  it.key("q");
  assert.equal(it.zoom(), 100);
  it.key("-", it.at("element"));
  assert.equal(it.zoom(), 100);
});

test("zooming stops at a tenth and at eight times", () => {
  const it = open();
  it.draw("svg", DRAWING);
  for (let step = 0; step < 40; step += 1) {
    it.click("out");
  }
  assert.equal(it.zoom(), 10);
  for (let step = 0; step < 60; step += 1) {
    it.click("in");
  }
  assert.equal(it.zoom(), 800);
});

test("the view controls report what was chosen", () => {
  const it = open();
  const view = it.at("view");
  view.value = "internal";
  view.dispatchEvent(new it.window.Event("change"));
  assert.deepEqual(it.last(), {
    command: "setView",
    view: "internal",
    element: "",
  });
  assert.equal(it.at("element").style.display, "inline");
});

test("a drawing arriving sets the controls to what is being shown", () => {
  const it = open();
  it.draw("svg", DRAWING, "internal", "Car");
  assert.equal(it.at("view").value, "internal");
  assert.equal(it.at("element").value, "Car");
  assert.equal(it.at("element").style.display, "inline");
  it.draw("svg", DRAWING, "browser", "");
  assert.equal(it.at("element").style.display, "none");
});

test("a message takes the place of a drawing and no room to zoom", () => {
  const it = open();
  it.draw("svg", DRAWING);
  it.draw("message", "Nothing to draw yet");
  assert.equal(it.canvas.textContent, "Nothing to draw yet");
  assert.equal(it.sizer.style.width, "0px");
  it.click("fit");
  assert.equal(it.zoom(), 100);
});

test("what is written out carries the colours that are on screen", () => {
  const it = open();
  it.draw(
    "svg",
    '<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">' +
      "<style>:root { --box: #ffffff; --line: #000000; }" +
      "@media (prefers-color-scheme: dark) { :root { --box: #1e1e1e; } }" +
      "</style><rect/></svg>"
  );
  // the query has already been settled by the window it is shown in
  const svg = it.canvas.querySelector("svg");
  svg.style.setProperty("--box", "#1e1e1e");
  svg.style.setProperty("--line", "#d4d4d4");

  const written = it.window.pinned(svg);
  const pin = written.slice(written.lastIndexOf(":root{"));
  assert.equal(pin, ":root{--box:#1e1e1e;--line:#d4d4d4;}</style></svg>");
  // and it comes after the query, so it is the rule that wins
  assert.ok(written.lastIndexOf(":root{") > written.indexOf("prefers-color-scheme"));
  // what was written is a copy: the drawing on screen is left alone
  assert.equal(it.canvas.querySelectorAll("style").length, 1);
});

test("a drawing that settled no colours is written out as it is", () => {
  const it = open();
  it.draw("svg", DRAWING);
  const written = it.window.pinned(it.canvas.querySelector("svg"));
  assert.ok(!written.includes(":root{"), written);
  assert.ok(written.includes("<rect"));
});

test("saving without a drawing still asks, so it can be refused by name", () => {
  const it = open();
  it.draw("message", "Nothing to draw yet");
  it.click("save");
  assert.deepEqual(it.last(), { command: "save" });
});

test("anything but a drawing is ignored", () => {
  const it = open();
  it.draw("svg", DRAWING);
  it.window.dispatchEvent(
    new it.window.MessageEvent("message", { data: { command: "something" } })
  );
  assert.equal(it.zoom(), 100);
  assert.equal(it.sizer.style.width, "800px");
});

test("the drawing is written into a page that lets nothing of its own run", () => {
  // The SVG arrives as markup and goes in with `innerHTML`. What keeps
  // a script in a model file out of this window is the policy, not the
  // care taken wherever the drawing was made.
  const html = page();
  const policy = html.match(
    /<meta http-equiv="Content-Security-Policy" content="([^"]+)">/
  );
  assert.ok(policy, html.slice(0, 200));
  assert.match(policy[1], /default-src 'none'/);
  assert.match(policy[1], /img-src data:/);
  assert.match(policy[1], /style-src 'unsafe-inline'/);

  // only the page's own script is let through, by a nonce of its own
  const nonce = policy[1].match(/script-src 'nonce-([A-Za-z0-9]+)'/);
  assert.ok(nonce, policy[1]);
  assert.ok(html.includes(`<script nonce="${nonce[1]}">`), "the script is not the one named");
  assert.notEqual(nonce[1], page().match(/script-src 'nonce-([A-Za-z0-9]+)'/)[1]);
});
