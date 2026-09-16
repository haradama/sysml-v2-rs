// Colouring the model, out of the grammar the VSCode extension ships.
//
// `editors/vscode/syntaxes/sysml.tmLanguage.json` run through
// `vscode-textmate` and `vscode-oniguruma` -- the engine VSCode itself
// tokenises with -- so what a reader sees here is what they see in the
// editor, and there is one grammar to keep right rather than two.
// `editors/vscode/test/grammar.test.mjs` is what keeps it right: it
// reads the reserved words straight out of the lexer's own table in
// `crates/sysml-syntax/src/kind.rs`.
//
// This is the one thing on the page that is not the language server.
// The server has no `textDocument/semanticTokens` to ask instead, and
// colouring is wanted on every keystroke rather than on every parse.

import * as oniguruma from "vscode-oniguruma";
import * as textmate from "vscode-textmate";

/// What a scope is painted as, innermost scope first, longest match
/// first. The names are the page's, not TextMate's: a theme here is
/// seven colours, and `keyword.declaration` and `keyword.control` are
/// both a keyword to a reader.
const PAINT: [string, string][] = [
  ["comment", "note"],
  // what a `doc` holds is the model's own prose, and the grammar is
  // careful to tell it from an aside; so is this
  ["string.quoted.other.documentation", "doc"],
  ["constant.character.escape", "escape"],
  ["string", "text"],
  ["constant.numeric", "number"],
  ["constant.language", "number"],
  ["entity.name.tag", "tag"],
  ["entity.name.namespace", "type"],
  ["entity.name.type", "type"],
  ["keyword.operator", "operator"],
  ["keyword", "keyword"],
  ["storage.modifier", "keyword"],
];

/// The three characters that would otherwise be markup. The painted
/// layer is written with `innerHTML`, so this is what stands between a
/// model and the page -- and `a_model_cannot_write_markup_into_the_page`
/// in `test/page.test.mjs` is what holds it to that.
function escape(text: string): string {
  return text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}

function painted(scopes: string[]): string | undefined {
  // innermost first: the outermost is always `source.sysml`
  for (let at = scopes.length - 1; at >= 0; at -= 1) {
    for (const [scope, paint] of PAINT) {
      if (scopes[at].startsWith(scope)) {
        return paint;
      }
    }
  }
  return undefined;
}

/// A model too long to colour.
///
/// The painted layer is one element's worth of HTML, so replacing it
/// lays the whole of it out again -- which costs about 0.7 ms per
/// thousand characters whatever the cache does, and the cache only
/// takes the tokenising off that. Measured in Chromium: 2.6 ms a
/// keystroke at 40 lines, 16 ms at 300, 52 ms at 1000.
///
/// A keystroke is not worth more than about the frame it lands in, and
/// this is where that runs out. Past it the text shows for itself, the
/// way it does before the grammar has loaded -- an editor with no
/// colour, rather than an editor that stutters. Colouring a model this
/// long would mean replacing one line of the painted layer rather than
/// all of it, which is a real editor's job and not this page's.
const TOO_LONG = 50_000;

/// One line, as it was last coloured.
///
/// A grammar reads a line in the state the line before it left, so a
/// line may be reused when its text and that state are both what they
/// were. Typing changes one line and leaves that state alone, so
/// everything under the caret is reused and what a keystroke costs stops
/// depending on how long the model is. Pressing return shifts every line
/// below it and costs a full pass, which is one keystroke in a line's
/// worth of them.
interface Line {
  text: string;
  before: textmate.StateStack;
  after: textmate.StateStack;
  html: string;
}

/// Load the grammar and hand back what paints a model with it.
export async function highlighter(
  grammar: string,
  onig: string
): Promise<(source: string) => string | undefined> {
  const [wasm, rules] = await Promise.all([
    fetch(onig).then((answered) => answered.arrayBuffer()),
    fetch(grammar).then((answered) => answered.text()),
  ]);
  await oniguruma.loadWASM(wasm);
  const registry = new textmate.Registry({
    onigLib: Promise.resolve({
      createOnigScanner: (patterns) => new oniguruma.OnigScanner(patterns),
      createOnigString: (line) => new oniguruma.OnigString(line),
    }),
    loadGrammar: async () => textmate.parseRawGrammar(rules, "sysml.json"),
  });
  const sysml = await registry.loadGrammar("source.sysml");
  if (!sysml) {
    throw new Error("the grammar did not load");
  }

  let last: Line[] = [];

  return (source: string) => {
    if (source.length > TOO_LONG) {
      last = [];
      return undefined;
    }
    const lines = source.split("\n");
    const painting: Line[] = [];
    const html: string[] = [];
    let stack = textmate.INITIAL;
    for (const [at, line] of lines.entries()) {
      const had = last[at];
      if (had && had.text === line && had.before.equals(stack)) {
        painting.push(had);
        html.push(had.html);
        stack = had.after;
        continue;
      }
      const { tokens, ruleStack } = sysml.tokenizeLine(line, stack);
      let out = "";
      for (const token of tokens) {
        const text = escape(line.slice(token.startIndex, token.endIndex));
        const paint = painted(token.scopes);
        out += paint ? `<span class="t-${paint}">${text}</span>` : text;
      }
      painting.push({ text: line, before: stack, after: ruleStack, html: out });
      html.push(out);
      stack = ruleStack;
    }
    last = painting;
    // One newline more than the source has. `<pre>` drops a single
    // trailing one, and the textarea does not -- so without this the
    // painted layer is a line short of the text it sits under, and the
    // last line of a model that ends in a newline is never painted.
    return `${html.join("\n")}\n`;
  };
}
