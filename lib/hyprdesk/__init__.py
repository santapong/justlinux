"""hyprdesk — shared library for the desktop widget fleet.

THE central place for parameters. Rules for every widget/script:
  · read settings ONLY via conf() / conf_get()   (~/.config/conky/widgets.conf)
  · read colors ONLY via colors()                 (theme-aware; never hardcode)
  · build layer surfaces ONLY via layer.LayerWindow

Conky-side equivalents live in ~/.config/conky/card.lua (colors/place/apply) —
if you change theme resolution here, mirror it there.
"""
from .theme import conf, conf_get, colors, rgb, css_rgba   # noqa: F401
