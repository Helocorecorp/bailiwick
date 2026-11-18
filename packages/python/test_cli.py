"""Smoke test of the packaged CLI against the repository's examples."""

import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def cli(*args):
    return subprocess.run([sys.executable, "-m", "bailiwick", *args], cwd=ROOT, capture_output=True, text=True)


def test_all():
    r = cli("explain", "examples/clinic-sovereign.bwk", "--catalog", "examples/catalog/all.bwk")
    assert r.returncode == 0, r.stderr
    assert "feasible: e-de-he-ck, f-de-he-ck" in r.stdout
    assert cli("check", "examples/storefront-eu-policy.bwk").returncode == 0
    r = cli("check-plan", "tests/fixtures/check-plan.bwk", "tests/fixtures/terraform-plan.json")
    assert r.returncode == 1 and "FAIL aws_s3_bucket.backups" in r.stdout
    r = cli("certify", "examples/clinic-sovereign.bwk", "--catalog", "examples/catalog/all.bwk", "--place", "api=e-de-he")
    assert r.returncode == 1 and "refusing to certify" in r.stderr
    assert cli("check", "nope.bwk").returncode == 2
    r = cli("prove", "examples/storefront.bwk", "--implies", "eu-sovereign")
    assert r.returncode == 1 and "REFUTED  service `web`" in r.stdout


if __name__ == "__main__":
    test_all()
    print("python package: all checks passed")
