"""Regenerates crates/platform/src/icons.rs from phosphor.css.

    python3 tools/fonts/icons.py name1 name2 ...   # adds icons to the module
"""
import re
import sys

ROOT = __file__.rsplit("/tools/", 1)[0]
css = open(f"{ROOT}/tools/fonts/phosphor.css").read()
table = dict(re.findall(r'\.ph\.ph-([a-z0-9-]+):before \{\s*content: "\\(\w+)";', css))
path = f"{ROOT}/crates/platform/src/icons.rs"
source = open(path).read()
for name in sys.argv[1:]:
    const = name.upper().replace("-", "_")
    if name not in table:
        print("unknown icon:", name)
    elif f"pub const {const}:" not in source:
        source += f'pub const {const}: &str = "\\u{{{table[name]}}}";\n'
open(path, "w").write(source)
