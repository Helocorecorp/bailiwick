// Bailiwick for Node: the real toolchain, compiled to WebAssembly.
//
//   const bwk = require("@helocore/bailiwick");
//   const r = bwk.run("explain", "bailiwick.bwk", { policies: ["gdpr"] });
//   process.stdout.write(r.stdout);   // r.exit: 0 ok, 1 blocked or errors, 2 usage
"use strict";

const fs = require("fs");
const path = require("path");

let instance;
function wasm() {
  if (!instance) {
    const bytes = fs.readFileSync(path.join(__dirname, "bailiwick.wasm"));
    instance = new WebAssembly.Instance(new WebAssembly.Module(bytes), {}).exports;
  }
  return instance;
}

function callWasm(fn, text) {
  const x = wasm();
  const bytes = new TextEncoder().encode(text);
  const p = x.bwk_alloc(bytes.length);
  new Uint8Array(x.memory.buffer, p, bytes.length).set(bytes);
  const r = x[fn](p, bytes.length);
  x.bwk_free(p, bytes.length);
  return JSON.parse(new TextDecoder().decode(new Uint8Array(x.memory.buffer, r, x.bwk_result_len())));
}

// Imports resolve relative to the importing file, exactly as the toolchain
// joins them; bare names (`import "gdpr"`) are built-in rule packs.
function joinLike(from, imp) {
  if (imp.startsWith("/")) return imp;
  const i = from.lastIndexOf("/");
  return i < 0 ? imp : from.slice(0, i) + "/" + imp;
}
const isPack = (s) => s.length > 0 && !/[/\\.]/.test(s);

// Read a Bailiwick file and everything it imports.
function collect(files, file, read) {
  if (file in files) return;
  let src;
  try {
    src = read(file);
  } catch {
    return; // reported by the toolchain as "cannot read"
  }
  files[file] = src;
  for (const m of src.matchAll(/^\s*import\s+"([^"]*)"/gm)) {
    if (!isPack(m[1])) collect(files, joinLike(file, m[1]), read);
  }
}

/**
 * Run a command as the native CLI would.
 * @param {"check"|"explain"|"fmt"|"certify"|"check-plan"|"prove"} command
 * @param {string} file the main .bwk file
 * @param {object} [opts] policies, catalog, plans, vendor, place, format ("text"|"json"|"markdown"), read
 */
function run(command, file, opts = {}) {
  const read = opts.read || ((p) => fs.readFileSync(p, "utf8"));
  const files = {};
  collect(files, file, read);
  for (const p of opts.policies || []) if (!isPack(p)) collect(files, p, read);
  if (opts.catalog) collect(files, opts.catalog, read);
  for (const p of opts.implies || []) if (!isPack(p)) collect(files, p, read);
  if (opts.baseline) collect(files, opts.baseline, read);
  for (const p of opts.plans || []) {
    try { files[p] = read(p); } catch { /* reported */ }
  }
  return callWasm("bwk_call", JSON.stringify({
    command,
    main: file,
    files,
    policies: opts.policies || [],
    catalog: opts.catalog || null,
    plans: opts.plans || [],
    vendor: opts.vendor || null,
    place: opts.place || [],
    implies: opts.implies || [],
    baseline: opts.baseline || null,
    format: opts.format || "text",
  }));
}

/** Evaluate one program held in memory (the playground's entry point). */
function evaluate(source) {
  return callWasm("bwk_run", source);
}

module.exports = { run, evaluate };
