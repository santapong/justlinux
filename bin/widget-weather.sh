#!/usr/bin/env python3
"""Weather (wttr.in, no API key) as conky markup. Cache 15 min."""
import json, os, time, urllib.request
from pathlib import Path

CACHE = Path(os.environ.get("XDG_RUNTIME_DIR", "/tmp")) / "widget-weather.cache"
if CACHE.exists() and time.time() - CACHE.stat().st_mtime < 900:
    print(CACHE.read_text(), end=""); raise SystemExit

ICONS = {"Sunny": "󰖙", "Clear": "󰖔", "Partly cloudy": "󰖕", "Cloudy": "󰖐",
         "Overcast": "󰖐", "Mist": "󰖑", "Fog": "󰖑", "Rain": "󰖗",
         "Light rain": "󰖗", "Heavy rain": "󰖖", "Thunder": "󰖓", "Snow": "󰖘"}

def icon(desc):
    for k, v in ICONS.items():
        if k.lower() in desc.lower():
            return v
    return "󰖕"

try:
    with urllib.request.urlopen("https://wttr.in/?format=j1", timeout=6) as r:
        d = json.loads(r.read())
    cur = d["current_condition"][0]
    def clean(s, n):
        s = str(s).replace("$", "$$").replace("#", "")
        return s if len(s) <= n else s[:n - 1] + "…"
    area = clean(d["nearest_area"][0]["areaName"][0]["value"], 16)
    desc = clean(cur["weatherDesc"][0]["value"], 22)
    days = d["weather"][:3]
    rows = [f"${{color1}}{icon(desc)}  {area}${{color}}${{alignr}}"
            f"${{color2}}{cur['temp_C']}°C${{color}}",
            f"{desc}${{alignr}}${{color3}}feels {cur['FeelsLikeC']}°C${{color}}",
            "${color3}${hr}${color}"]
    names = ("today", "tomorrow", "day after")
    for name, day in zip(names, days):
        rows.append(f"${{color3}}{name}${{color}}${{alignr}}"
                    f"{day['mintempC']}–{day['maxtempC']}°C · "
                    f"${{color2}}{day['hourly'][4]['chanceofrain']}%󰖗${{color}}")
    text = "\n".join(rows)
    CACHE.write_text(text)
except Exception:
    text = (CACHE.read_text() + "\n${color3}(stale)${color}") \
        if CACHE.exists() else "${color3}weather unavailable${color}"
print(text)
