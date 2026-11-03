#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Simeon H.K. Fitch
"""Run F-Engrave's batch mode with stubbed GUI modules. usage: run_batch.py <f-engrave.py> <args...>"""
import os, runpy, sys
stub = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, stub)
src = sys.argv[1]
sys.argv = [src] + sys.argv[2:]
runpy.run_path(src, run_name="__main__")
