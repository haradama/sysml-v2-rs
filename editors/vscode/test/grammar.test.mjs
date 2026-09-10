// The TextMate grammar, run through the engine VSCode itself tokenises
// with, so what is checked here is the colouring a reader will see.

import assert from "node:assert/strict";
import { readdir, readFile } from "node:fs/promises";
import { createRequire } from "node:module";
import test from "node:test";

// both are CommonJS, and what they export is on the module itself
const require = createRequire(import.meta.url);
const oniguruma = require("vscode-oniguruma");
const textmate = require("vscode-textmate");

const GRAMMAR = new URL("../syntaxes/sysml.tmLanguage.json", import.meta.url);

await oniguruma.loadWASM(
  (await readFile(require.resolve("vscode-oniguruma/release/onig.wasm"))).buffer
);
const registry = new textmate.Registry({
  onigLib: Promise.resolve({
    createOnigScanner: (patterns) => new oniguruma.OnigScanner(patterns),
    createOnigString: (line) => new oniguruma.OnigString(line),
  }),
  loadGrammar: async () =>
    textmate.parseRawGrammar(await readFile(GRAMMAR, "utf8"), "sysml.json"),
});
const grammar = await registry.loadGrammar("source.sysml");

/// Every token of `source` as `[text, innermost scope]`, which is what a
/// theme colours by.
function tokens(source) {
  let stack = textmate.INITIAL;
  const out = [];
  for (const line of source.split("\n")) {
    const { tokens: found, ruleStack } = grammar.tokenizeLine(line, stack);
    stack = ruleStack;
    for (const token of found) {
      const text = line.slice(token.startIndex, token.endIndex);
      if (text.trim() !== "") {
        out.push([text.trim(), token.scopes[token.scopes.length - 1]]);
      }
    }
  }
  return out;
}

/// The scope over the first token holding `text`, which the tokeniser
/// may have split from what follows it.
function scopeOf(source, text) {
  const found = tokens(source).find(([had]) => had.includes(text));
  assert.ok(found, `no token \`${text}\` in ${JSON.stringify(tokens(source))}`);
  return found[1];
}

test("what a `doc` holds is not colored as an aside", () => {
  // The prose in a `doc` is the model's own -- an element with a body,
  // which `sysml export` writes out and hover shows -- while a comment
  // beside it is skipped on the way to the model. Reading alike, they
  // used to look alike.
  const source =
    "package P {\n\tdoc\n\t/* what this is for. */\n" +
    "\t/* an aside nobody models */\n\t// and a note\n}\n";
  assert.equal(
    scopeOf(source, "what this is for"),
    "string.quoted.other.documentation.sysml"
  );
  assert.equal(
    scopeOf(source, "an aside nobody models"),
    "comment.block.sysml"
  );
  assert.equal(scopeOf(source, "// and a note"), "comment.line.double-slash.sysml");
  assert.equal(scopeOf(source, "doc"), "keyword.declaration.sysml");
});

test("a doc body written on the keyword's own line is the same thing", () => {
  const inline = "part def A { doc /* one line */ }\n";
  assert.equal(scopeOf(inline, "one line"), "string.quoted.other.documentation.sysml");
  // and the name a `doc` may carry is a name, not prose
  const named = "doc theReason /* why */\n";
  assert.equal(scopeOf(named, "theReason"), "entity.name.type.sysml");
  assert.equal(scopeOf(named, "why"), "string.quoted.other.documentation.sysml");
});

test("what a `comment` element holds is a comment, because it is one", () => {
  const source = "package P {\n\tcomment C about P /* said about it */\n}\n";
  assert.equal(scopeOf(source, "said about it"), "comment.block.sysml");
});

test("an annotation is a tag, whichever mark introduces it", () => {
  assert.equal(scopeOf("@Safety;\n", "@Safety"), "entity.name.tag.metadata.sysml");
  assert.equal(scopeOf("#approved\n", "#approved"), "entity.name.tag.metadata.sysml");
  assert.equal(
    scopeOf("@Safety::Level;\n", "@Safety::Level"),
    "entity.name.tag.metadata.sysml"
  );
});

test("a declaration names something, and a feature says what it is", () => {
  assert.equal(scopeOf("package P;\n", "P"), "entity.name.type.sysml");
  assert.equal(scopeOf("part def Wheel;\n", "Wheel"), "entity.name.type.sysml");
  assert.equal(scopeOf("part w : Wheel;\n", "Wheel"), "entity.name.type.sysml");
  assert.equal(scopeOf("part def Car :> Vehicle;\n", "Vehicle"), "entity.name.type.sysml");
  assert.equal(scopeOf("part x :>> ISQ::mass;\n", "ISQ::mass"), "entity.name.type.sysml");
  assert.equal(scopeOf("part w subsets wheel;\n", "wheel"), "entity.name.type.sysml");
  assert.equal(scopeOf("part a defined by B;\n", "B"), "entity.name.type.sysml");
  // a short name stands before the name a declaration goes by, and both
  // of them name the same thing
  const short = "verification def <'T.1'> VerifyIt;\n";
  assert.equal(scopeOf(short, "VerifyIt"), "entity.name.type.sysml");
  assert.equal(scopeOf(short, "'T.1'"), "entity.name.type.sysml");
  // and an import names a path rather than a type
  assert.equal(scopeOf("import P::Q::*;\n", "import"), "keyword.control.sysml");
  assert.equal(scopeOf("import P::Q::*;\n", "P::Q"), "entity.name.namespace.sysml");
  assert.equal(scopeOf("import all P::*;\n", "all"), "storage.modifier.sysml");
  assert.equal(scopeOf("import all P::*;\n", "P"), "entity.name.namespace.sysml");
});

/// The words SysML reserves, read off the lexer's own table.
///
/// A keyword reserved in neither notation is an ordinary name as well --
/// `metadata def <effect> EffectMetadata` in the corpus names one -- so
/// only what the table marks reserved here is a keyword out of place
/// when a rule colours it as a name.
async function reserved() {
  const table = await readFile(
    new URL("../../../crates/sysml-syntax/src/kind.rs", import.meta.url),
    "utf8"
  );
  const words = new Set();
  for (const [, word, where] of table.matchAll(
    /\("([a-z]+)",\s*[A-Z_]+_KW,\s*Reserved::(\w+)\)/g
  )) {
    if (where === "Both" || where === "SysML") {
      words.add(word);
    }
  }
  assert.ok(words.size > 100, `only ${words.size} reserved word(s) read`);
  return words;
}

test("no reserved word anywhere in the corpus is coloured as a name", async () => {
  // The rules that colour a name read what is around it rather than what
  // it is, and a rule that reaches too far takes the keyword after it.
  const root = new URL(
    "../../../vendor/sysml-v2-release/sysml/src/",
    import.meta.url
  );
  let entries;
  try {
    entries = await readdir(root, { recursive: true, withFileTypes: true });
  } catch {
    console.log("skipping: the corpus submodule is not checked out");
    return;
  }
  const words = await reserved();
  const found = [];
  let read = 0;
  for (const entry of entries) {
    if (!entry.isFile() || !entry.name.endsWith(".sysml")) {
      continue;
    }
    read += 1;
    const file = new URL(`${entry.parentPath}/${entry.name}`, "file://");
    for (const [text, scope] of tokens(await readFile(file, "utf8"))) {
      if (scope.startsWith("entity.name") && words.has(text)) {
        found.push(`${entry.name}: \`${text}\` as ${scope}`);
      }
    }
  }
  assert.ok(read > 100, `only ${read} corpus file(s) read`);
  assert.deepEqual(found.slice(0, 12), []);
});
