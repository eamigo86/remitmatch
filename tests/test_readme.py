"""Run the README example and compare its output with the documented output."""

import contextlib
import io
import re
from pathlib import Path

README = Path(__file__).resolve().parent.parent / "README.md"
EXAMPLE = re.compile(
    r"```python\n(?P<code>.*?)```\s*\n\s*Output:\s*\n\s*```text\n(?P<out>.*?)```", re.S
)


def test_readme_example_runs_and_prints_the_documented_output() -> None:
    """The Python example in the README runs and prints exactly the block after it."""
    found = EXAMPLE.search(README.read_text(encoding="utf-8"))
    assert found is not None, "README needs a python example followed by an Output text block"
    buffer = io.StringIO()
    with contextlib.redirect_stdout(buffer):
        exec(compile(found["code"], str(README), "exec"), {"__name__": "__readme__"})
    assert buffer.getvalue() == found["out"]
