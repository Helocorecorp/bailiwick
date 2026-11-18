#!/usr/bin/env node
// `npx @helocore/bailiwick explain bailiwick.bwk --policy gdpr`
"use strict";

const fs = require("fs");
const { run } = require("..");

const HELP = `bailiwick: say what your software needs, not where it runs

  bailiwick check      <file> [--policy <p>]...
  bailiwick explain    <file> [--catalog <offers>] [--policy <p>]... [--json|--markdown]
  bailiwick fmt        <file> [--write]
  bailiwick check-plan <file> <plan.json | manifests.yaml>... [--vendor <v>] [--policy <p>]... [--markdown]
  bailiwick certify    <file> [--catalog <offers>] [--place <unit=offer,...>]...
  bailiwick prove      <file> [--implies <p>]... [--no-looser-than <old.bwk>]   (for every possible offer)

--policy takes a file or a built-in rule pack: gdpr, eu-sovereign, uk-gdpr, pipeda,
canada-residency, quebec-law25, hipaa, ccpa, lgpd, pipl, rbi-payments, my-health-records.
The native toolchain (cargo install --git https://github.com/Helocorecorp/bailiwick)
adds init, verify, import, ir and the language server.`;

const argv = process.argv.slice(2);
const command = argv.shift();
if (!command || command === "help" || command === "--help" || command === "-h") {
  console.log(HELP);
  process.exit(command ? 0 : 2);
}

const opts = { policies: [], place: [], plans: [], implies: [] };
const positional = [];
let write = false;
for (let i = 0; i < argv.length; i++) {
  const a = argv[i];
  const value = () => {
    if (i + 1 >= argv.length) { console.error(`bailiwick: ${a} needs a value`); process.exit(2); }
    return argv[++i];
  };
  if (a === "--policy") opts.policies.push(value());
  else if (a === "--catalog") opts.catalog = value();
  else if (a === "--place") opts.place.push(value());
  else if (a === "--vendor") opts.vendor = value();
  else if (a === "--implies") opts.implies.push(value());
  else if (a === "--no-looser-than") opts.baseline = value();
  else if (a === "--json") opts.format = "json";
  else if (a === "--markdown") opts.format = "markdown";
  else if (a === "--write") write = true;
  else if (a.startsWith("--")) { console.error(`bailiwick: unknown flag \`${a}\``); process.exit(2); }
  else positional.push(a);
}
const [file, ...rest] = positional;
if (!file) { console.error(`bailiwick ${command}: expected a file`); process.exit(2); }
if (command === "check-plan") opts.plans = rest;

const r = run(command, file, opts);
if (r.stderr) process.stderr.write(r.stderr);
if (command === "fmt" && write && r.exit === 0) {
  fs.writeFileSync(file, r.stdout);
  console.log(`formatted: ${file}`);
} else if (r.stdout) {
  process.stdout.write(r.stdout);
}
process.exit(r.exit);
