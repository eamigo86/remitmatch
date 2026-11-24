"""Check that the type stubs match the runtime."""

import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def test_stubs_match_the_runtime() -> None:
    """Stubtest finds no difference between the stubs and the installed package."""
    completed = subprocess.run(
        [
            sys.executable,
            "-m",
            "mypy.stubtest",
            "--mypy-config-file",
            "pyproject.toml",
            "remitmatch",
        ],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=False,
    )
    assert completed.returncode == 0, completed.stdout + completed.stderr
