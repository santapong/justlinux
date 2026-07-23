"""gtk-layer-shell base window for desktop widgets.

The GIR typelib for gtk-layer-shell isn't packaged on this box, so the C
library is driven through ctypes (PyCapsule __gpointer__ trick).
"""
import ctypes
import os

import gi
gi.require_version("Gtk", "3.0")
from gi.repository import Gtk  # noqa: E402

from .theme import conf  # noqa: E402

LLS = ctypes.CDLL("libgtk-layer-shell.so.0")
ctypes.pythonapi.PyCapsule_GetPointer.restype = ctypes.c_void_p
ctypes.pythonapi.PyCapsule_GetPointer.argtypes = [ctypes.py_object,
                                                  ctypes.c_char_p]

EDGE = {"left": 0, "right": 1, "top": 2, "bottom": 3}
LAYER = {"background": 0, "bottom": 1, "top": 2, "overlay": 3}
POS_ANCHORS = {
    "top_left": ("top", "left"), "top_middle": ("top",),
    "top_right": ("top", "right"), "middle_left": ("left",),
    "middle_right": ("right",), "bottom_left": ("bottom", "left"),
    "bottom_middle": ("bottom",), "bottom_right": ("bottom", "right"),
}


def gptr(widget):
    return ctypes.c_void_p(
        ctypes.pythonapi.PyCapsule_GetPointer(widget.__gpointer__, None))


class LayerWindow(Gtk.Window):
    """A layer-shell surface positioned from widgets.conf.

    name:  widget key — position read from <name>_pos / <name>_x / <name>_y
    layer: 'bottom' (desktop, default) / 'top' / 'overlay'
           (env HYPRPET_LAYER-style overrides are the caller's business)
    """

    KEYBOARD = {"none": 0, "exclusive": 1, "on_demand": 2}

    def __init__(self, name, default_pos="bottom_middle", default_x=0,
                 default_y=12, layer="bottom", namespace=None,
                 keyboard="none", fullscreen=False):
        super().__init__()
        visual = self.get_screen().get_rgba_visual()
        if visual:
            self.set_visual(visual)
        c = conf()
        p = gptr(self)
        LLS.gtk_layer_init_for_window(p)
        LLS.gtk_layer_set_layer(p, LAYER.get(layer, 1))
        LLS.gtk_layer_set_namespace(
            p, (namespace or f"hypr-{name}").encode())
        LLS.gtk_layer_set_keyboard_mode(p, self.KEYBOARD.get(keyboard, 0))
        if fullscreen:
            for edge in EDGE.values():
                LLS.gtk_layer_set_anchor(p, edge, True)
            # ignore exclusive zones (waybar) — cover the WHOLE monitor
            LLS.gtk_layer_set_exclusive_zone(p, -1)
        else:
            pos = c.get(f"{name}_pos", default_pos)
            try:                        # hand-edited conf must never crash
                x = int(c.get(f"{name}_x", default_x))
            except (TypeError, ValueError):
                x = int(default_x)
            try:
                y = int(c.get(f"{name}_y", default_y))
            except (TypeError, ValueError):
                y = int(default_y)
            for edge in POS_ANCHORS.get(pos, ("bottom",)):
                LLS.gtk_layer_set_anchor(p, EDGE[edge], True)
                LLS.gtk_layer_set_margin(
                    p, EDGE[edge], x if edge in ("left", "right") else y)
        if not fullscreen:
            LLS.gtk_layer_set_exclusive_zone(p, 0)

    def set_target_monitor(self, gdk_monitor):
        """Pin the surface to a specific monitor (before show_all)."""
        if gdk_monitor is not None:
            LLS.gtk_layer_set_monitor(gptr(self), gptr(gdk_monitor))

    def anchor_all(self, edges=("left", "bottom")):
        p = gptr(self)
        for e in edges:
            LLS.gtk_layer_set_anchor(p, EDGE[e], True)
