"""Tests for the package version and the reconcile placeholder."""

from importlib.metadata import version

import pytest
import remitmatch
from remitmatch import _core


def test_version_is_consistent() -> None:
    """The package version matches the installed metadata and the native module."""
    assert remitmatch.__version__ == version("remitmatch")
    assert remitmatch.__version__ == _core.version()


def test_reconcile_is_not_implemented() -> None:
    """Reconcile raises until the matching engine ships in 0.1.0."""
    with pytest.raises(NotImplementedError, match="0.1.0"):
        remitmatch.reconcile([], [])
