// The preview's page: the controls round the drawing, and the script that
// zooms, pans and rasterises whatever is posted into it.
//
// Nothing here knows about VSCode beyond `acquireVsCodeApi`, so the page
// can be loaded into a plain DOM and driven the way a reader drives it.

/// The page a preview panel is opened with, once, for the life of the
/// panel: the drawings are posted in rather than written into it.
export function page(): string {
  return `<!DOCTYPE html>
<html>
<head>
<meta charset="utf-8">
<style>
  html, body { height: 100%; }
  body {
    margin: 0; display: flex; flex-direction: column; overflow: hidden;
    font-family: var(--vscode-font-family); font-size: 12px;
    /* named rather than left to the host, because a saved PNG is painted
       on this colour and a transparent one would save pale lines onto
       nothing at all */
    background: var(--vscode-editor-background, #ffffff);
  }
  #bar {
    flex: none; display: flex; gap: 0.4em; align-items: center;
    padding: 0.4em 0.5em;
    border-bottom: 1px solid var(--vscode-panel-border, transparent);
  }
  select, input, button {
    background: var(--vscode-input-background); color: var(--vscode-input-foreground);
    border: 1px solid var(--vscode-input-border, transparent); padding: 2px 6px;
  }
  button { cursor: pointer; }
  button:hover { background: var(--vscode-toolbar-hoverBackground, var(--vscode-input-background)); }
  #zoom {
    min-width: 3.6em; text-align: center; font-variant-numeric: tabular-nums;
    color: var(--vscode-descriptionForeground);
  }
  .spacer { flex: 1; }
  #diagram { flex: 1; overflow: auto; cursor: grab; }
  #diagram.panning { cursor: grabbing; }
  #canvas { transform-origin: 0 0; }
  #canvas svg { display: block; max-width: none; }
  #canvas p { padding: 0.5em; }
</style>
</head>
<body>
<div id="bar">
  <select id="view" title="What the diagram shows">
    <option value="definitions">Definitions</option>
    <option value="internal">Internal structure</option>
    <option value="browser">Tree</option>
  </select>
  <input id="element" placeholder="element name" style="display:none">
  <span class="spacer"></span>
  <button id="out" title="Zoom out (-)">&minus;</button>
  <span id="zoom">100%</span>
  <button id="in" title="Zoom in (+)">+</button>
  <button id="reset" title="Actual size (0)">1:1</button>
  <button id="fit" title="Fit to window">Fit</button>
  <button id="save" title="Save the diagram as an image">Save&hellip;</button>
</div>
<div id="diagram"><div id="sizer"><div id="canvas"></div></div></div>
<script>
  const vscode = acquireVsCodeApi();
  const view = document.getElementById("view");
  const element = document.getElementById("element");
  const diagram = document.getElementById("diagram");
  const sizer = document.getElementById("sizer");
  const canvas = document.getElementById("canvas");
  const readout = document.getElementById("zoom");

  const LEAST = 0.1, MOST = 8, STEP = 1.2;
  /// The size the drawing is drawn at, which is what a zoom is a factor of.
  let natural = { width: 0, height: 0 };
  let scale = (vscode.getState() || {}).scale || 1;

  function apply() {
    canvas.style.transform = "scale(" + scale + ")";
    sizer.style.width = natural.width * scale + "px";
    sizer.style.height = natural.height * scale + "px";
    readout.textContent = Math.round(scale * 100) + "%";
    vscode.setState({ scale: scale });
  }

  /// Zoom so that whatever is under (x, y) on screen stays under it.
  function zoomAt(wanted, x, y) {
    const next = Math.min(MOST, Math.max(LEAST, wanted));
    const box = diagram.getBoundingClientRect();
    const held = { x: x - box.left + diagram.scrollLeft, y: y - box.top + diagram.scrollTop };
    const ratio = next / scale;
    scale = next;
    apply();
    diagram.scrollLeft = held.x * ratio - (x - box.left);
    diagram.scrollTop = held.y * ratio - (y - box.top);
  }

  /// Zoom about the middle of the window, which is what a button means.
  function zoomBy(factor) {
    const box = diagram.getBoundingClientRect();
    zoomAt(scale * factor, box.left + box.width / 2, box.top + box.height / 2);
  }

  function fit() {
    if (!natural.width || !natural.height) {
      return;
    }
    const room = 16;
    scale = Math.min(
      MOST,
      Math.max(
        LEAST,
        Math.min(
          (diagram.clientWidth - room) / natural.width,
          (diagram.clientHeight - room) / natural.height
        )
      )
    );
    apply();
    diagram.scrollLeft = 0;
    diagram.scrollTop = 0;
  }

  document.getElementById("in").addEventListener("click", () => zoomBy(STEP));
  document.getElementById("out").addEventListener("click", () => zoomBy(1 / STEP));
  document.getElementById("fit").addEventListener("click", fit);
  document.getElementById("reset").addEventListener("click", () => {
    scale = 1;
    apply();
  });

  // ctrl (or command) and the wheel zooms towards the pointer; the wheel
  // by itself keeps scrolling, which is what a long diagram wants
  diagram.addEventListener("wheel", (event) => {
    if (!event.ctrlKey && !event.metaKey) {
      return;
    }
    event.preventDefault();
    zoomAt(scale * Math.pow(1.0015, -event.deltaY), event.clientX, event.clientY);
  }, { passive: false });

  let panning = null;
  diagram.addEventListener("mousedown", (event) => {
    if (event.button !== 0) {
      return;
    }
    panning = {
      x: event.clientX, y: event.clientY,
      left: diagram.scrollLeft, top: diagram.scrollTop,
    };
    diagram.classList.add("panning");
    event.preventDefault();
  });
  window.addEventListener("mousemove", (event) => {
    if (!panning) {
      return;
    }
    diagram.scrollLeft = panning.left - (event.clientX - panning.x);
    diagram.scrollTop = panning.top - (event.clientY - panning.y);
  });
  window.addEventListener("mouseup", () => {
    panning = null;
    diagram.classList.remove("panning");
  });

  window.addEventListener("keydown", (event) => {
    if (event.target === element) {
      return; // a name is being typed, and a minus sign is part of one
    }
    if (event.key === "+" || event.key === "=") {
      zoomBy(STEP);
    } else if (event.key === "-") {
      zoomBy(1 / STEP);
    } else if (event.key === "0") {
      scale = 1;
      apply();
    } else {
      return;
    }
    event.preventDefault();
  });

  function send() {
    element.style.display = view.value === "internal" ? "inline" : "none";
    vscode.postMessage({ command: "setView", view: view.value, element: element.value });
  }
  view.addEventListener("change", send);
  element.addEventListener("change", send);

  /// The drawing as it is on screen, written out so it can be rasterised.
  ///
  /// The SVG carries a palette for either theme and chooses between them
  /// with a media query. Once it is loaded back as an image it is a
  /// document of its own, and which way that query goes there is the
  /// engine's business -- so the colours it resolved to on screen are
  /// pinned onto the copy, and what is saved is what is being looked at.
  function pinned(svg) {
    const shown = getComputedStyle(svg);
    const names = new Set();
    svg.querySelectorAll("style").forEach((sheet) => {
      const declarations = sheet.textContent.match(/--[\\w-]+(?=\\s*:)/g) || [];
      declarations.forEach((name) => names.add(name));
    });
    const settled = Array.from(names)
      .map((name) => [name, shown.getPropertyValue(name).trim()])
      .filter(([, colour]) => colour !== "");
    const copy = svg.cloneNode(true);
    if (settled.length > 0) {
      const pin = document.createElementNS("http://www.w3.org/2000/svg", "style");
      // last, and outside any query, so it is the one that wins
      pin.textContent =
        ":root{" +
        settled.map(([name, colour]) => name + ":" + colour + ";").join("") +
        "}";
      copy.appendChild(pin);
    }
    return new XMLSerializer().serializeToString(copy);
  }

  /// The drawing as PNG, at twice its size so the text stays readable.
  ///
  /// The page's own background is painted behind it: the drawing has none
  /// of its own, and a dark theme's pale lines would otherwise be saved
  /// onto nothing at all.
  function rasterise(svg) {
    return new Promise((resolve, reject) => {
      const source = pinned(svg);
      const image = new Image();
      image.onload = () => {
        const ratio = 2;
        const page = document.createElement("canvas");
        page.width = Math.max(1, Math.round(natural.width * ratio));
        page.height = Math.max(1, Math.round(natural.height * ratio));
        const pen = page.getContext("2d");
        pen.scale(ratio, ratio);
        pen.fillStyle =
          getComputedStyle(document.body).backgroundColor || "#ffffff";
        pen.fillRect(0, 0, natural.width, natural.height);
        pen.drawImage(image, 0, 0, natural.width, natural.height);
        try {
          resolve(page.toDataURL("image/png").split(",")[1]);
        } catch (refused) {
          reject(refused);
        }
      };
      image.onerror = () => reject(new Error("the drawing did not load"));
      image.src =
        "data:image/svg+xml;charset=utf-8," + encodeURIComponent(source);
    });
  }

  document.getElementById("save").addEventListener("click", () => {
    const svg = canvas.querySelector("svg");
    if (!svg) {
      vscode.postMessage({ command: "save" });
      return;
    }
    rasterise(svg).then(
      (png) => vscode.postMessage({ command: "save", png: png }),
      () => vscode.postMessage({ command: "save" })
    );
  });

  window.addEventListener("message", (event) => {
    const drawn = event.data;
    if (drawn.command !== "draw") {
      return;
    }
    view.value = drawn.view;
    element.value = drawn.element;
    element.style.display = drawn.view === "internal" ? "inline" : "none";
    if (drawn.kind === "svg") {
      canvas.innerHTML = drawn.body;
      const svg = canvas.querySelector("svg");
      natural = {
        width: parseFloat(svg.getAttribute("width")) || svg.clientWidth,
        height: parseFloat(svg.getAttribute("height")) || svg.clientHeight,
      };
    } else {
      canvas.textContent = "";
      canvas.appendChild(document.createElement("p")).textContent = drawn.body;
      natural = { width: 0, height: 0 };
    }
    apply();
  });

  vscode.postMessage({ command: "ready" });
</script>
</body>
</html>`;
}
