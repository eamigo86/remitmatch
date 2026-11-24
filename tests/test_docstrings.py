"""Check that no docstring contains a backtick."""

import ast
from pathlib import Path

from remitmatch import _core

ROOT = Path(__file__).resolve().parent.parent
SOURCES = sorted(
    path
    for folder in ("python", "tests")
    for pattern in ("*.py", "*.pyi")
    for path in (ROOT / folder).rglob(pattern)
)
DOCUMENTED = (ast.Module, ast.ClassDef, ast.FunctionDef, ast.AsyncFunctionDef)


def test_source_docstrings_have_no_backticks() -> None:
    """Every docstring in the Python sources and stubs is plain text."""
    offenders = []
    for path in SOURCES:
        for node in ast.walk(ast.parse(path.read_text(encoding="utf-8"))):
            if isinstance(node, DOCUMENTED) and "`" in (ast.get_docstring(node) or ""):
                offenders.append(f"{path.relative_to(ROOT)}:{getattr(node, 'lineno', 1)}")
    assert not offenders, offenders


def test_native_docstrings_have_no_backticks() -> None:
    """The docstrings exposed by the native module are plain text."""
    assert "`" not in (_core.__doc__ or "")
    assert "`" not in (_core.version.__doc__ or "")
