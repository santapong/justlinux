"""Phone-style placement grid for the desktop card fleet.

Each monitor divides into grid_cols × grid_rows cells (widgets.conf,
defaults 12×6) inside a fixed outer margin. A widget occupies its
top-left cell plus an auto span of ceil(size/cell); cell coordinates are
resolution-proof and make overlap a set-intersection check instead of
pixel math. The bottom strip (pet walkway + dock) stays out of the grid
via BOTTOM_RESERVE.

COORDINATE SPACE: every function takes WORKAREA dimensions (monitor size
minus Hyprland's reserved insets — waybar reserves 30px top here) and
returns WORKAREA-relative pixels. Layer-shell margins are workarea-
relative, so grid origins map 1:1 onto gtk_layer_set_margin values; a
caller working in absolute monitor pixels (hyprctl layers) must subtract
the reserved offsets first.
"""
import math

from .theme import conf

MARGIN = 24          # outer gutter, px (matches the fleet's classic offsets)
BOTTOM_RESERVE = 68  # pet strip + dock clearance at the screen bottom


def geometry(mon_w, mon_h, c=None):
    """(cols, rows, cell_w, cell_h) for a monitor, from widgets.conf."""
    c = conf() if c is None else c
    try:
        cols = max(2, min(24, int(c.get("grid_cols", 12))))
        rows = max(2, min(16, int(c.get("grid_rows", 6))))
    except (TypeError, ValueError):
        cols, rows = 12, 6
    cell_w = (mon_w - 2 * MARGIN) / cols
    cell_h = (mon_h - MARGIN - BOTTOM_RESERVE) / rows
    return cols, rows, cell_w, cell_h


def origin(mon_w, mon_h, col, row, c=None):
    """Top-left pixel of a cell (monitor-local)."""
    cols, rows, cw, ch = geometry(mon_w, mon_h, c)
    col = max(0, min(cols - 1, int(col)))
    row = max(0, min(rows - 1, int(row)))
    return round(MARGIN + col * cw), round(MARGIN + row * ch)


def cell_at(mon_w, mon_h, x, y, c=None):
    """Nearest cell (col, row) for a monitor-local pixel position."""
    cols, rows, cw, ch = geometry(mon_w, mon_h, c)
    col = round((x - MARGIN) / cw)
    row = round((y - MARGIN) / ch)
    return max(0, min(cols - 1, col)), max(0, min(rows - 1, row))


def span(mon_w, mon_h, w, h, c=None):
    """(col_span, row_span) cells a w×h px widget occupies."""
    _, _, cw, ch = geometry(mon_w, mon_h, c)
    return max(1, math.ceil(w / cw)), max(1, math.ceil(h / ch))


def clamp(mon_w, mon_h, col, row, col_span, row_span, c=None):
    """Clamp a cell position so the whole span stays on the grid."""
    cols, rows, _, _ = geometry(mon_w, mon_h, c)
    return (max(0, min(cols - col_span, int(col))),
            max(0, min(rows - row_span, int(row))))


def cells_of(col, row, col_span, row_span):
    """The set of (col, row) cells covered — overlap = set intersection."""
    return {(col + i, row + j)
            for i in range(col_span) for j in range(row_span)}
