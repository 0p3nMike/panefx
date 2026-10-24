#!/usr/bin/env python3
"""Run c4nvas's `bake` command where GTK 4, GStreamer and gtk4-layer-shell are not installed.

usage: c4nvas-bake.py C4NVAS bake [NAME|all] [PERCENT ...|all]

c4nvas imports Gtk, Gtk4LayerShell and Gst at the top, but its bake path draws only with
cairo and Pango and pipes the frames into ffmpeg; the three are used by the live window
alone. So they are replaced here by placeholder modules that accept any attribute, and
c4nvas (the file named by C4NVAS) is then run unchanged as the main program.
"""
import runpy
import sys
import types

import gi

PLACEHOLDERS = ("Gtk", "Gtk4LayerShell", "Gst")
_require_version = gi.require_version


def require_version(namespace, version):
    if namespace not in PLACEHOLDERS:
        _require_version(namespace, version)


class _Anything:
    """stands in for any class, function or constant of a placeholder module."""

    def __getattr__(self, name):
        return _Anything()

    def __call__(self, *args, **kwargs):
        return _Anything()


# c4nvas imports Pango and PangoCairo without naming a version; naming it here keeps
# PyGObject from warning about that on every run.
_require_version("Pango", "1.0")
_require_version("PangoCairo", "1.0")
gi.require_version = require_version
for _ns in PLACEHOLDERS:
    _mod = types.ModuleType("gi.repository." + _ns)
    _mod.__getattr__ = lambda name: _Anything()
    sys.modules["gi.repository." + _ns] = _mod

if len(sys.argv) < 3 or sys.argv[2] != "bake":
    sys.exit(__doc__.split("\n\n")[1])
path = sys.argv[1]
sys.argv = [path] + sys.argv[2:]
runpy.run_path(path, run_name="__main__")
