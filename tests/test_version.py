"""Tests for the package version."""

from importlib.metadata import version

import remitmatch
from packaging.version import Version
from remitmatch import _core


def test_version_is_consistent() -> None:
    """The package version is the installed metadata and agrees with the native module."""
    assert remitmatch.__version__ == version("remitmatch")
    assert Version(remitmatch.__version__) == Version(_core.version())


def test_version_is_a_canonical_python_version() -> None:
    """The package version is written in the canonical Python form, pre-releases included."""
    assert remitmatch.__version__ == str(Version(remitmatch.__version__))
