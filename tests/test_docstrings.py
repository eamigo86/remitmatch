"""Check that public items are documented and no docstring contains a backtick."""

import ast
import inspect
from enum import Enum
from pathlib import Path

import remitmatch
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


def test_public_objects_are_documented_in_plain_text() -> None:
    """Every exported object and public member has a non-empty docstring without backticks."""
    assert remitmatch.__doc__
    missing = []
    for name in remitmatch.__all__:
        if name.startswith("__"):
            continue
        obj = getattr(remitmatch, name)
        targets = {name: obj}
        if inspect.isclass(obj) and not issubclass(obj, Enum):
            targets.update(
                {
                    f"{name}.{member}": value
                    for member, value in vars(obj).items()
                    if not member.startswith("_")
                }
            )
        for label, target in targets.items():
            doc = inspect.getdoc(target) or ""
            if not doc.strip() or "`" in doc:
                missing.append(label)
    assert not missing, missing
