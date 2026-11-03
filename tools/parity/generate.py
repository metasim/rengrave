#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Simeon H.K. Fitch
"""Regenerates the parity fixtures by running F-Engrave's batch mode.

usage: generate.py <path to f-engrave.py>

For each line of cases.txt this writes <name>.cfg, the settings given to the original program,
and <name>.ngc, its G-code output, to crates/rengrave_core/tests/parity. The original needs no
display or pyclipper here: run_batch.py substitutes stub modules, which the batch path allows.
"""
import os
import subprocess
import sys

here = os.path.dirname(os.path.abspath(__file__))
out_dir = os.path.join(here, "..", "..", "crates", "rengrave_core", "tests", "parity")


def main():
    source = os.path.abspath(sys.argv[1])
    with open(os.path.join(here, "cases.txt")) as cases:
        for raw in cases:
            line = raw.strip()
            if not line or line.startswith("#"):
                continue
            name, text, options = [p.strip() for p in line.split(" | ")] if line.count(" | ") == 2 \
                else [p.strip() for p in (line + " ").split(" | ")]
            cfg = ['(fengrave_set fontdir "." )', '(fengrave_set fontfile "tiny.cxf" )']
            for option in filter(None, (o.strip() for o in options.split(";"))):
                key, value = option.split(" ", 1)
                if key in ("imagefile", "fontfile", "fontdir"):
                    value = '"%s"' % value
                cfg.append("(fengrave_set %s %s )" % (key, value))
            cfg_path = os.path.join(out_dir, name + ".cfg")
            with open(cfg_path, "w") as f:
                f.write("\n".join(cfg) + "\n")
            args = [sys.executable, "-I", os.path.join(here, "run_batch.py"), source,
                    "-g", name + ".cfg", "-b"]
            if text != "-":
                args += ["-t", text]
            done = subprocess.run(args, cwd=out_dir, capture_output=True, text=True, check=True)
            lines = done.stdout.split("\n")
            start = lines.index("(F-Engrave Batch Mode)") + 1
            with open(os.path.join(out_dir, name + ".ngc"), "w") as f:
                f.write("\n".join(lines[start:]))
            with open(os.path.join(out_dir, name + ".text"), "w") as f:
                f.write(text)
            print(name, len(lines) - start, "lines", done.stderr.strip()[:200])


main()
