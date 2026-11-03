"""Minimal stand-in for tkinter so F-Engrave's batch path runs without a display."""
TRUE, FALSE = True, False
ALL = "all"
LEFT = RIGHT = TOP = BOTTOM = BOTH = X = Y = W = E = N = S = NW = SW = CENTER = SUNKEN = FLAT = END = NONE = ""
RAISED = GROOVE = RIDGE = SOLID = ""
DISABLED = NORMAL = ""


class _Var:
    def __init__(self, *a, **k):
        self._v = None

    def set(self, v):
        self._v = v

    def get(self):
        return self._v

    def trace_add(self, *a):
        return "x"

    def trace_remove(self, *a):
        pass


class StringVar(_Var):
    def get(self):
        return "" if self._v is None else str(self._v)


class BooleanVar(_Var):
    def set(self, v):
        self._v = bool(int(v)) if isinstance(v, str) else bool(v)

    def get(self):
        return bool(self._v)


class IntVar(_Var):
    pass


class _Widget:
    def __init__(self, *a, **k):
        pass

    def __getattr__(self, name):
        def anything(*a, **k):
            return None

        return anything


class Tk(_Widget):
    pass


class Frame(_Widget):
    def __init__(self, master=None, **k):
        self.master = master


class Toplevel(_Widget):
    pass


Label = Button = Canvas = Entry = Text = Scrollbar = Listbox = Menu = Checkbutton = Radiobutton = OptionMenu = _Widget
PhotoImage = _Widget
