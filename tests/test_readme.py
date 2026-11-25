"""Run the README example and compare its output with the documented output."""

import contextlib
import io
import re
from pathlib import Path

import pytest

README = Path(__file__).resolve().parent.parent / "README.md"
EXAMPLE = re.compile(
    r"```python\n(?P<code>.*?)```\s*\n\s*Output:\s*\n\s*```text\n(?P<out>.*?)```", re.S
)


EXAMPLES = [
    (found["code"], found["out"]) for found in EXAMPLE.finditer(README.read_text(encoding="utf-8"))
]


def test_readme_has_the_object_and_the_arrow_examples() -> None:
    """The README holds at least two python examples, each followed by an Output text block."""
    assert len(EXAMPLES) >= 2


@pytest.mark.parametrize(("code", "expected"), EXAMPLES)
def test_readme_example_runs_and_prints_the_documented_output(code: str, expected: str) -> None:
    """Each Python example in the README runs and prints exactly the block after it.

    Args:
        code: Source of the example.
        expected: Output documented after the example.
    """
    buffer = io.StringIO()
    with contextlib.redirect_stdout(buffer):
        exec(compile(code, str(README), "exec"), {"__name__": "__readme__"})
    assert buffer.getvalue() == expected
