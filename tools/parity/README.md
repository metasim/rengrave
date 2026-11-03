# Parity harness

Runs F-Engrave's batch mode without a display, to produce the reference programs used by
`crates/rengrave_core/tests/parity.rs`.

| File | Purpose |
| --- | --- |
| `generate.py` | Runs every case and writes the fixtures |
| `cases.txt` | The cases: name, text, and settings |
| `run_batch.py` | Runs the original with the stubs below on its module path |
| `tkinter/` | Just enough of `tkinter` for the batch path to import and run |
| `pyclipper.py` | An empty stand-in; cleanup paths are not reachable from batch mode |

Get the F-Engrave source from <https://www.scorchworks.com/Fengrave/fengrave.html>, then:

```bash
python3 tools/parity/generate.py /path/to/F-Engrave-1.78_src/f-engrave.py
```

Each case writes `<name>.cfg`, `<name>.text`, and `<name>.ngc` to
`crates/rengrave_core/tests/parity`. To add a case, add a line to `cases.txt`, regenerate, and add
its name to the `parity_cases!` list in the test.
