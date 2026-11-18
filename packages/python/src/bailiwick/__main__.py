"""`bailiwick explain bailiwick.bwk --policy gdpr` (or `python -m bailiwick ...`)."""

from __future__ import annotations

import sys
from pathlib import Path

from . import run

HELP = """bailiwick: say what your software needs, not where it runs

  bailiwick check      <file> [--policy <p>]...
  bailiwick explain    <file> [--catalog <offers>] [--policy <p>]... [--json|--markdown]
  bailiwick fmt        <file> [--write]
  bailiwick check-plan <file> <plan.json | manifests.yaml>... [--vendor <v>] [--policy <p>]... [--markdown]
  bailiwick certify    <file> [--catalog <offers>] [--place <unit=offer,...>]...
  bailiwick prove      <file> [--implies <p>]... [--no-looser-than <old.bwk>]   (for every possible offer)

--policy takes a file or a built-in rule pack: gdpr, eu-sovereign, uk-gdpr, pipeda,
canada-residency, quebec-law25, hipaa, ccpa, lgpd, pipl, rbi-payments, my-health-records.
The native toolchain (cargo install --git https://github.com/Helocorecorp/bailiwick)
adds init, verify, import, ir and the language server."""


def main(argv=None) -> int:
    args = list(sys.argv[1:] if argv is None else argv)
    if not args or args[0] in ("help", "--help", "-h"):
        print(HELP)
        return 0 if args else 2
    command = args.pop(0)
    policies, place, positional, implies = [], [], [], []
    catalog = vendor = baseline = None
    fmt_out, write = "text", False
    i = 0
    while i < len(args):
        a = args[i]
        if a in ("--policy", "--catalog", "--place", "--vendor", "--implies", "--no-looser-than"):
            if i + 1 >= len(args):
                print(f"bailiwick: {a} needs a value", file=sys.stderr)
                return 2
            v = args[i + 1]
            i += 1
            if a == "--policy":
                policies.append(v)
            elif a == "--catalog":
                catalog = v
            elif a == "--place":
                place.append(v)
            elif a == "--implies":
                implies.append(v)
            elif a == "--no-looser-than":
                baseline = v
            else:
                vendor = v
        elif a == "--json":
            fmt_out = "json"
        elif a == "--markdown":
            fmt_out = "markdown"
        elif a == "--write":
            write = True
        elif a.startswith("--"):
            print(f"bailiwick: unknown flag `{a}`", file=sys.stderr)
            return 2
        else:
            positional.append(a)
        i += 1
    if not positional:
        print(f"bailiwick {command}: expected a file", file=sys.stderr)
        return 2
    file, rest = positional[0], positional[1:]
    r = run(command, file, policies=policies, catalog=catalog, plans=rest if command == "check-plan" else (),
            vendor=vendor, place=place, implies=implies, baseline=baseline, format=fmt_out)
    if r.stderr:
        sys.stderr.write(r.stderr)
    if command == "fmt" and write and r.exit == 0:
        Path(file).write_text(r.stdout, encoding="utf-8")
        print(f"formatted: {file}")
    elif r.stdout:
        sys.stdout.write(r.stdout)
    return r.exit


if __name__ == "__main__":
    sys.exit(main())
