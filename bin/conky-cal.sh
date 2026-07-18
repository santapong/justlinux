#!/usr/bin/env python3
"""This month's calendar as conky markup: header accented, today highlighted.
(Python because `cal` isn't installed on this box.)"""
import calendar
from datetime import date

t = date.today()
cal = calendar.TextCalendar(firstweekday=6)          # Sunday first, like cal(1)
lines = cal.formatmonth(t.year, t.month).rstrip("\n").split("\n")

out = [f"${{color1}}{lines[0]}${{color}}",           # "     July 2026"
       f"${{color3}}{lines[1]}${{color}}"]           # "Su Mo Tu We Th Fr Sa"

day = f"{t.day:2}"
for row in lines[2:]:
    # each day occupies a fixed 2-char cell, so pad-match to hit only today
    padded = f" {row} "
    hl = padded.replace(f" {day} ", f" ${{color2}}{day}${{color3}} ", 1)
    body = hl[1:-1] if hl != padded else row
    out.append(f"${{color3}}{body}${{color}}")
print("\n".join(out))
