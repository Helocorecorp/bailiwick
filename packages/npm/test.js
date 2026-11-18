// Smoke test of the packaged CLI against the repository's examples.
"use strict";
const { execFileSync, spawnSync } = require("child_process");
const path = require("path");
const assert = require("assert");
const root = path.join(__dirname, "..", "..");
const cli = (...args) => spawnSync(process.execPath, [path.join(__dirname, "bin", "bailiwick.js"), ...args], { cwd: root, encoding: "utf8" });

let r = cli("explain", "examples/clinic-sovereign.bwk", "--catalog", "examples/catalog/all.bwk");
assert.strictEqual(r.status, 0, r.stderr);
assert.match(r.stdout, /feasible: e-de-he-ck, f-de-he-ck/);

r = cli("check", "examples/storefront-eu-policy.bwk");
assert.strictEqual(r.status, 0, r.stderr + r.stdout);

r = cli("explain", "examples/storefront.bwk", "--catalog", "examples/catalog/all.bwk", "--policy", "eu-sovereign", "--markdown");
assert.match(r.stdout, /Policies in force: `eu-sovereign`/);

r = cli("check-plan", "tests/fixtures/check-plan.bwk", "tests/fixtures/terraform-plan.json");
assert.strictEqual(r.status, 1);
assert.match(r.stdout, /FAIL aws_s3_bucket.backups/);

r = cli("check-plan", "tests/fixtures/check-plan.bwk", "tests/fixtures/k8s-placed.yaml", "--vendor", "vendor-e");
assert.match(r.stdout, /StatefulSet\/customers \(us-east-1\)/);

r = cli("certify", "examples/clinic-sovereign.bwk", "--catalog", "examples/catalog/all.bwk", "--place", "api=e-de-he");
assert.strictEqual(r.status, 1);
assert.match(r.stderr, /refusing to certify/);

r = cli("check", "nope.bwk");
assert.strictEqual(r.status, 2);

r = cli("prove", "examples/storefront.bwk", "--implies", "eu-sovereign");
assert.strictEqual(r.status, 1);
assert.match(r.stdout, /REFUTED  service `web`/);
r = cli("prove", "examples/clinic-sovereign.bwk");
assert.strictEqual(r.status, 0, r.stdout + r.stderr);

console.log("npm package: all checks passed");
