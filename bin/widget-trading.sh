#!/usr/bin/env python3
"""Trading card: price, 24h change and a sparkline chart per coin.
Coins from widgets.conf `trading_coins` (CoinGecko ids, comma-separated,
default bitcoin,ethereum). Cache 5 min."""
import json, os, sys, time, urllib.request
from pathlib import Path

sys.path.insert(0, str(Path.home() / ".local/lib"))
from hyprdesk import conf_get

FIELDS = "--fields" in sys.argv
if "--coins" in sys.argv:
    _i = sys.argv.index("--coins")
    _coins_arg = sys.argv[_i + 1] if len(sys.argv) > _i + 1 else ""
else:
    _coins_arg = conf_get("trading_coins", "bitcoin,ethereum")
COINS = [c.strip() for c in _coins_arg.split(",") if c.strip()]

RUN = Path(os.environ.get("XDG_RUNTIME_DIR", "/tmp"))
# param-hash cache: two instances with different coins must NOT share —
# a fixed cache file would silently show the same coins on both cards
import hashlib
_tag = hashlib.md5(",".join(COINS).encode()).hexdigest()[:8]
CACHE = RUN / (f"widget-trading-{_tag}.fields" if FIELDS
               else "widget-trading.cache")
# fields TTL < host interval (300): TTL == interval means the cache is
# always fresh at tick time, silently halving the real refresh rate
_TTL = 240 if FIELDS else 300
if CACHE.exists() and time.time() - CACHE.stat().st_mtime < _TTL:
    print(CACHE.read_text(), end=""); raise SystemExit

SPARK = "▁▂▃▄▅▆▇█"
SYM = {"bitcoin": "󰠓 BTC", "ethereum": "󰡪 ETH", "solana": "◎ SOL",
       "dogecoin": "Ð DOGE", "cardano": "₳ ADA"}


def spark(values, width=22):
    if not values:
        return ""
    step = max(1, len(values) // width)
    pts = [values[i] for i in range(0, len(values), step)][:width]
    lo, hi = min(pts), max(pts)
    rng = (hi - lo) or 1
    return "".join(SPARK[round((v - lo) / rng * 7)] for v in pts)


def fetch(url):
    req = urllib.request.Request(url, headers={"User-Agent": "hyprdesk-widget"})
    with urllib.request.urlopen(req, timeout=8) as r:
        return json.loads(r.read())


try:
    ids = ",".join(COINS)
    prices = fetch("https://api.coingecko.com/api/v3/simple/price"
                   f"?ids={ids}&vs_currencies=usd&include_24hr_change=true")
    if FIELDS:
        lines, n = [], 0
        deadline = time.time() + 10   # stay well inside the host's 15s kill
        for coin in COINS:
            p = prices.get(coin)
            if not p:
                continue
            pts = []
            if time.time() < deadline:
                try:
                    chart = fetch("https://api.coingecko.com/api/v3/coins/"
                                  f"{coin}/market_chart"
                                  "?vs_currency=usd&days=1")
                    vals = [v for _t, v in chart.get("prices", [])]
                    step = max(1, len(vals) // 40)
                    pts = [f"{v:.2f}" for v in vals[::step]][:40]
                except Exception:
                    pass              # price row still renders, no spark
            chg = p.get("usd_24h_change") or 0
            price = p["usd"]
            ptxt = f"${price:,.0f}" if price >= 100 else f"${price:,.2f}"
            lines += [f"coin.{n}.sym={SYM.get(coin, coin[:6].upper())}",
                      f"coin.{n}.price={ptxt}",
                      f"coin.{n}.change={'+' if chg >= 0 else '-'}"
                      f"{abs(chg):.1f}%",
                      f"coin.{n}.spark={','.join(pts)}"]
            n += 1
        if not lines:                 # every id unknown: that's a FAILURE —
            sys.exit(1)               # a blank-but-fresh card masks it
        text = "\n".join(lines)
        CACHE.write_text(text)
        print(text)
        raise SystemExit
    rows = ["${color1}󰋂  MARKETS${color}${alignr}${color3}24 h${color}",
            "${color3}${hr}${color}"]
    for coin in COINS:
        p = prices.get(coin)
        if not p:
            continue
        chart = fetch(f"https://api.coingecko.com/api/v3/coins/{coin}"
                      "/market_chart?vs_currency=usd&days=1")
        line = spark([v for _t, v in chart.get("prices", [])])
        chg = p.get("usd_24h_change") or 0
        cc = "color4" if chg >= 0 else "color5"
        arrow = "󰁝" if chg >= 0 else "󰁅"
        price = p["usd"]
        ptxt = f"{price:,.0f}" if price >= 100 else f"{price:,.2f}"
        rows.append(f"{SYM.get(coin, coin[:6].upper())}${{alignr}}"
                    f"${{color2}}$${ptxt}${{color}}  "
                    f"${{{cc}}}{arrow}{abs(chg):.1f}%${{color}}")
        rows.append(f"${{color3}}{line}${{color}}")
    text = "\n".join(rows)
    CACHE.write_text(text)
except SystemExit:
    raise
except Exception:
    if FIELDS:
        sys.exit(1)          # host keeps last fields + shows (stale)
    text = (CACHE.read_text() + "${alignr}${color3}(stale)${color}") \
        if CACHE.exists() else "${color3}󰋂 market data unreachable${color}"
print(text)
