"""Tests for the package version."""

from importlib.metadata import version

import remitmatch
from remitmatch import _core


def test_version_is_consistent() -> None:
    """The package version matches the installed metadata and the native module."""
    assert remitmatch.__version__ == version("remitmatch")
    assert remitmatch.__version__ == _core.version()
