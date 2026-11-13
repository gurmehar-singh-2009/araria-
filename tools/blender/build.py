"""Rebuilds every model in `assets/models` and the model manifest.

    blender -b -P tools/blender/build.py            # everything
    blender -b -P tools/blender/build.py -- props   # one module
"""

import os
import sys

sys.path.insert(0, os.path.dirname(__file__))

import lib  # noqa: E402

wanted = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else ["props", "character"]
for module in wanted:
    __import__(module).build()
lib.write_manifest()
