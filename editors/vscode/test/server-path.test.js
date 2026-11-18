// Which server the extension runs, without VS Code: `vscode` is stubbed.
const assert = require("assert");
const fs = require("fs");
const os = require("os");
const path = require("path");
const Module = require("module");
const load = Module._load;
Module._load = (req, ...rest) => (req === "vscode" ? {} : req === "vscode-languageclient/node" ? {} : load(req, ...rest));
const { serverCommand } = require("../extension.js");

const ext = fs.mkdtempSync(path.join(os.tmpdir(), "bwk-ext-"));
assert.strictEqual(serverCommand(ext, "bailiwick", "linux"), "bailiwick", "nothing bundled: the PATH");
fs.mkdirSync(path.join(ext, "server"));
fs.writeFileSync(path.join(ext, "server", "bailiwick"), "");
fs.writeFileSync(path.join(ext, "server", "bailiwick.exe"), "");
assert.strictEqual(serverCommand(ext, "bailiwick", "linux"), path.join(ext, "server", "bailiwick"), "bundled wins over the default");
assert.strictEqual(serverCommand(ext, "bailiwick", "win32"), path.join(ext, "server", "bailiwick.exe"), "Windows uses .exe");
assert.strictEqual(serverCommand(ext, "/opt/bw/bailiwick", "linux"), "/opt/bw/bailiwick", "a user's path wins");
console.log("server-path: ok");
