#!/usr/bin/env python3
"""Markets card: price, 24h change and a sparkline per symbol.

Two kinds of symbol, one card:
  · crypto  — CoinGecko ids (bitcoin, ethereum, solana …) via `trading_coins`
  · stocks  — ticker symbols (AAPL, NVDA, ^GSPC, BTC-USD …) via `trading_stocks`

Both feeds are keyless. Rows are capped at MAX_ROWS because the card is a
glance, not a portfolio — past ~7 rows it stops being readable and starts
being a table, and every extra symbol is another HTTP round trip inside the
host's 15s kill window. Anything over the cap is dropped and SAID SO in the
badge rather than silently vanishing. Cache 5 min."""
import json, os, sys, time, urllib.parse, urllib.request
from pathlib import Path

sys.path.insert(0, str(Path.home() / ".local/lib"))
from hyprdesk import conf_get

MAX_ROWS = 7          # hard ceiling; 5-6 is the comfortable range


def _arg(flag, conf_key, default):
    if flag in sys.argv:
        i = sys.argv.index(flag)
        return sys.argv[i + 1] if len(sys.argv) > i + 1 else ""
    return conf_get(conf_key, default)


FIELDS = "--fields" in sys.argv
COINS = [c.strip() for c in
         _arg("--coins", "trading_coins", "bitcoin,ethereum").split(",")
         if c.strip()]
STOCKS = [s.strip().upper() for s in
          _arg("--stocks", "trading_stocks", "").split(",") if s.strip()]
# crypto first, then stocks, then the cut — a deterministic order beats
# whichever feed answered first
_want = [("coin", c) for c in COINS] + [("stock", s) for s in STOCKS]
DROPPED = max(0, len(_want) - MAX_ROWS)
WANT = _want[:MAX_ROWS]
COINS = [s for k, s in WANT if k == "coin"]
STOCKS = [s for k, s in WANT if k == "stock"]

RUN = Path(os.environ.get("XDG_RUNTIME_DIR", "/tmp"))
# param-hash cache: two instances with different coins must NOT share —
# a fixed cache file would silently show the same coins on both cards
import hashlib
_tag = hashlib.md5(",".join(COINS + STOCKS).encode()).hexdigest()[:8]
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


def fetch(url, ua="hyprdesk-widget"):
    req = urllib.request.Request(url, headers={"User-Agent": ua})
    with urllib.request.urlopen(req, timeout=8) as r:
        return json.loads(r.read())


def stock(ticker):
    """(price, pct_change, spark_points) for one ticker, or None.

    Yahoo's chart endpoint is keyless and returns the day's closes, so one
    call gives the price, the previous close to diff against, and the
    sparkline — no second round trip like the crypto path needs. It wants a
    browser UA; the default one gets refused.
    """
    d = fetch("https://query1.finance.yahoo.com/v8/finance/chart/"
              f"{urllib.parse.quote(ticker)}?range=1d&interval=15m",
              ua="Mozilla/5.0 (X11; Linux x86_64)")
    res = (d.get("chart") or {}).get("result") or []
    if not res:
        return None
    meta = res[0].get("meta") or {}
    price = meta.get("regularMarketPrice")
    prev = meta.get("chartPreviousClose") or meta.get("previousClose")
    if price is None:
        return None
    try:
        closes = [c for c in
                  res[0]["indicators"]["quote"][0]["close"] if c is not None]
    except (KeyError, IndexError, TypeError):
        closes = []
    pct = ((price - prev) / prev * 100) if prev else 0.0
    return price, pct, closes


try:
    # ONE call for every coin — price, 24h change and the sparkline together.
    # The old shape asked /simple/price once and then /market_chart PER COIN,
    # which is an N+1 against a rate-limited free tier: the extra calls got
    # throttled and every crypto row silently rendered "no data" while the
    # stocks (one call each) kept their charts.
    markets = []
    if COINS:
        markets = fetch("https://api.coingecko.com/api/v3/coins/markets"
                        f"?vs_currency=usd&ids={','.join(COINS)}"
                        "&sparkline=true&price_change_percentage=24h")
    by_id = {c.get("id"): c for c in markets if isinstance(c, dict)}
    prices = {k: {"usd": v.get("current_price"),
                  "usd_24h_change": v.get("price_change_percentage_24h")}
              for k, v in by_id.items()}
    if FIELDS:
        lines, n = [], 0
        deadline = time.time() + 10   # stay well inside the host's 15s kill
        for coin in COINS:            # keep the user's configured order
            c = by_id.get(coin)
            if not c or c.get("current_price") is None:
                continue
            vals = (c.get("sparkline_in_7d") or {}).get("price") or []
            step = max(1, len(vals) // 40)
            pts = [f"{v:.2f}" for v in vals[::step]][:40]
            chg = c.get("price_change_percentage_24h") or 0
            price = c["current_price"]
            ptxt = f"${price:,.0f}" if price >= 100 else f"${price:,.2f}"
            lines += [f"coin.{n}.sym={SYM.get(coin, coin[:6].upper())}",
                      f"coin.{n}.price={ptxt}",
                      f"coin.{n}.change={'+' if chg >= 0 else '-'}"
                      f"{abs(chg):.1f}%",
                      f"coin.{n}.spark={','.join(pts)}"]
            n += 1
        for tk in STOCKS:
            if time.time() > deadline:
                break                 # out of budget: the rest wait a tick
            try:
                got = stock(tk)
            except Exception:
                got = None
            if not got:
                continue
            price, chg, closes = got
            step = max(1, len(closes) // 40)
            pts = [f"{v:.2f}" for v in closes[::step]][:40]
            ptxt = f"${price:,.0f}" if price >= 100 else f"${price:,.2f}"
            lines += [f"coin.{n}.sym={tk}",
                      f"coin.{n}.price={ptxt}",
                      f"coin.{n}.change={'+' if chg >= 0 else '-'}"
                      f"{abs(chg):.1f}%",
                      f"coin.{n}.spark={','.join(pts)}"]
            n += 1
        # the badge carries the truth about the cap: a silently short card
        # reads as "the market is quiet", not "we dropped three of yours"
        lines.append(f"cap={'+%d over' % DROPPED if DROPPED else '24 h'}")
        if n == 0:                    # every symbol unknown: that's a FAILURE —
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
