"""Bailiwick for Python: the real toolchain, compiled to WebAssembly.

    import bailiwick
    r = bailiwick.run("explain", "bailiwick.bwk", policies=["gdpr"])
    print(r.stdout)     # r.exit: 0 ok, 1 blocked or errors, 2 usage
"""

from __future__ import annotations

import json
import re
from dataclasses import dataclass
from pathlib import Path
from typing import Callable, Optional, Sequence

__all__ = ["Result", "run", "evaluate"]

_WASM = Path(__file__).with_name("bailiwick.wasm")
_instance = None


@dataclass
class Result:
    exit: int
    stdout: str
    stderr: str


def _exports():
    global _instance
    if _instance is None:
        import wasmtime

        store = wasmtime.Store()
        module = wasmtime.Module.from_file(store.engine, str(_WASM))
        inst = wasmtime.Instance(store, module, [])
        _instance = (store, inst.exports(store))
    return _instance


def _call(fn: str, text: str) -> dict:
    store, x = _exports()
    data = text.encode("utf-8")
    mem = x["memory"]
    p = x["bwk_alloc"](store, len(data))
    mem.write(store, data, p)
    r = x[fn](store, p, len(data))
    x["bwk_free"](store, p, len(data))
    n = x["bwk_result_len"](store)
    return json.loads(bytes(mem.read(store, r, r + n)).decode("utf-8"))


def _join_like(frm: str, imp: str) -> str:
    # Imports resolve relative to the importing file, as the toolchain joins them.
    if imp.startswith("/"):
        return imp
    i = frm.rfind("/")
    return imp if i < 0 else frm[:i] + "/" + imp


def _is_pack(s: str) -> bool:
    return bool(s) and not re.search(r"[/\\.]", s)


def _collect(files: dict, file: str, read: Callable[[str], str]) -> None:
    if file in files:
        return
    try:
        src = read(file)
    except OSError:
        return  # reported by the toolchain as "cannot read"
    files[file] = src
    for m in re.finditer(r'^\s*import\s+"([^"]*)"', src, re.M):
        if not _is_pack(m.group(1)):
            _collect(files, _join_like(file, m.group(1)), read)


def run(
    command: str,
    file: str,
    *,
    policies: Sequence[str] = (),
    catalog: Optional[str] = None,
    plans: Sequence[str] = (),
    vendor: Optional[str] = None,
    place: Sequence[str] = (),
    implies: Sequence[str] = (),
    baseline: Optional[str] = None,
    format: str = "text",
    read: Optional[Callable[[str], str]] = None,
) -> Result:
    """Run `check`, `explain`, `fmt`, `certify`, `check-plan` or `prove` as the native CLI would."""
    read = read or (lambda p: Path(p).read_text(encoding="utf-8"))
    files: dict = {}
    _collect(files, file, read)
    for p in policies:
        if not _is_pack(p):
            _collect(files, p, read)
    if catalog:
        _collect(files, catalog, read)
    for p in implies:
        if not _is_pack(p):
            _collect(files, p, read)
    if baseline:
        _collect(files, baseline, read)
    for p in plans:
        try:
            files[p] = read(p)
        except OSError:
            pass
    out = _call(
        "bwk_call",
        json.dumps(
            {
                "command": command,
                "main": file,
                "files": files,
                "policies": list(policies),
                "catalog": catalog,
                "plans": list(plans),
                "vendor": vendor,
                "place": list(place),
                "implies": list(implies),
                "baseline": baseline,
                "format": format,
            }
        ),
    )
    return Result(int(out["exit"]), out["stdout"], out["stderr"])


def evaluate(source: str) -> dict:
    """Evaluate one program held in memory: diagnostics, plan, certificate."""
    return _call("bwk_run", source)
