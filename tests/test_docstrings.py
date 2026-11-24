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


def _public_stub_items(
    body: list[ast.stmt], prefix: str
) -> dict[str, ast.ClassDef | ast.FunctionDef]:
    """Collect the public documented definitions of a stub body.

    Args:
        body: Statements of a module or class in a stub.
        prefix: Dotted path of the owner, empty for the module.

    Returns:
        Definitions keyed by dotted name, with nested class members included.
    """
    items: dict[str, ast.ClassDef | ast.FunctionDef] = {}
    for node in body:
        if isinstance(node, (ast.ClassDef, ast.FunctionDef)) and not node.name.startswith("_"):
            items[prefix + node.name] = node
            if isinstance(node, ast.ClassDef):
                items.update(_public_stub_items(node.body, f"{prefix}{node.name}."))
    return items


def test_stub_docstrings_equal_runtime_docstrings() -> None:
    """Each public stub item has the runtime docstring, and no runtime item lacks a stub."""
    tree = ast.parse((ROOT / "python" / "remitmatch" / "_core.pyi").read_text(encoding="utf-8"))
    items = _public_stub_items(tree.body, "")
    assert inspect.cleandoc(ast.get_docstring(tree, clean=False) or "") == inspect.cleandoc(
        _core.__doc__ or ""
    )
    problems = []
    for name, node in items.items():
        target: object = _core
        for part in name.split("."):
            target = vars(target)[part] if inspect.isclass(target) else getattr(target, part)
        stub_doc = inspect.cleandoc(ast.get_docstring(node, clean=False) or "")
        if not stub_doc or stub_doc != inspect.cleandoc(getattr(target, "__doc__", None) or ""):
            problems.append(f"docstring differs: {name}")
    runtime_names = {n for n in vars(_core) if not n.startswith("_")}
    for class_name in {n for n in items if "." not in n and inspect.isclass(getattr(_core, n))}:
        runtime_names |= {
            f"{class_name}.{n}" for n in vars(getattr(_core, class_name)) if not n.startswith("_")
        }
    problems += [f"missing from stub: {n}" for n in sorted(runtime_names - set(items))]
    assert not problems, problems
