#!/usr/bin/env python3
"""Now Playing via MPRIS over D-Bus (no playerctl needed)."""
FALLBACK = "${color3}󰝛 nothing playing${color}"
try:
    from gi.repository import Gio, GLib

    BUS = Gio.bus_get_sync(Gio.BusType.SESSION, None)

    def call(dest, iface, method, params=None,
             path="/org/mpris/MediaPlayer2"):
        return BUS.call_sync(dest, path, iface, method, params, None,
                             Gio.DBusCallFlags.NONE, 1000, None)

    def esc(s):
        # execpi re-parses output — a '$' in a track title is an injection
        return str(s).replace("$", "$$")

    names = call("org.freedesktop.DBus", "org.freedesktop.DBus", "ListNames",
                 path="/org/freedesktop/DBus").unpack()[0]
    players = [n for n in names if n.startswith("org.mpris.MediaPlayer2.")]
    out = FALLBACK
    for pl in players:
        try:
            def prop(name):
                return call(pl, "org.freedesktop.DBus.Properties", "Get",
                            GLib.Variant("(ss)",
                                         ("org.mpris.MediaPlayer2.Player",
                                          name))).unpack()[0]
            status = prop("PlaybackStatus")
            if status not in ("Playing", "Paused"):
                continue
            meta = prop("Metadata")
            title = esc(str(meta.get("xesam:title", "?"))[:34])
            artist = esc(", ".join(meta.get("xesam:artist", []))[:30])
            ic = "󰐊" if status == "Playing" else "󰏤"
            # org.mpris.MediaPlayer2.chromium.instance2 -> "chromium"
            app = esc(pl[len("org.mpris.MediaPlayer2."):].split(".")[0][:12])
            out = (f"${{color1}}{ic}  ${{color2}}{title}${{color}}\n"
                   f"${{color3}}{artist or '—'}${{color}}${{alignr}}"
                   f"${{color3}}{app}${{color}}")
            if status == "Playing":
                break
        except Exception:
            continue
    print(out)
except Exception:
    print(FALLBACK)
