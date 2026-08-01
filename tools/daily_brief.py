#!/usr/bin/env python3
"""bitbank daily brief — 商い状況ダイジェストを1回で出す低トークン版。

生ローソク足JSONをLLMに読ませず、指標・出来高統計をここで計算して
20〜30行のダイジェストだけを標準出力に出す。毎日これを1回叩けばよい。

使い方:
  python daily_brief.py                     # 既定ペア(btc_jpy eth_jpy xrp_jpy)
  python daily_brief.py btc_jpy eth_jpy     # ペア指定
  BITBANK_BRIEF_PAIRS="btc_jpy sol_jpy" python daily_brief.py

依存: `bitbank` CLI が PATH にあること。Python 3.9+。
"""
import sys, os, io, json, subprocess, datetime, statistics
from datetime import timezone, timedelta

# Windows でも日本語を壊さない
try:
    sys.stdout.reconfigure(encoding="utf-8")
except Exception:
    sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8")

JST = timezone(timedelta(hours=9))
WD = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]

def fetch(pair, tf, limit):
    cmd = ["bitbank", "candles", pair, f"--type={tf}", f"--limit={limit}",
           "--format=json", "--machine"]
    out = subprocess.run(cmd, capture_output=True, text=True, encoding="utf-8")
    if out.returncode != 0:
        raise RuntimeError(f"{pair} {tf}: {out.stderr.strip() or out.stdout.strip()}")
    j = json.loads(out.stdout)
    if not j.get("success"):
        raise RuntimeError(f"{pair} {tf}: envelope success=false")
    return j["data"], j.get("meta", {})

def sma(v, n):
    return sum(v[-n:]) / n if len(v) >= n else None

def ema(v, n):
    k = 2 / (n + 1); e = v[0]; out = [e]
    for x in v[1:]:
        e = x * k + e * (1 - k); out.append(e)
    return out

def rsi(c, n=14):
    if len(c) < n + 1: return None
    g = [max(c[i] - c[i-1], 0) for i in range(1, len(c))]
    l = [max(c[i-1] - c[i], 0) for i in range(1, len(c))]
    ag, al = sum(g[:n]) / n, sum(l[:n]) / n
    for i in range(n, len(g)):
        ag = (ag * (n-1) + g[i]) / n; al = (al * (n-1) + l[i]) / n
    return 100.0 if al == 0 else 100 - 100 / (1 + ag / al)

def macd(c):
    if len(c) < 35: return None
    e12, e26 = ema(c, 12), ema(c, 26)
    line = [a - b for a, b in zip(e12, e26)]
    sig = ema(line[25:], 9)
    return line[-1], sig[-1], line[-1] - sig[-1]

def atr(rows, n=14):
    if len(rows) < n + 1: return None
    trs = []
    for i in range(1, len(rows)):
        h, l, pc = rows[i]["high"], rows[i]["low"], rows[i-1]["close"]
        trs.append(max(h - l, abs(h - pc), abs(l - pc)))
    a = sum(trs[:n]) / n
    for i in range(n, len(trs)):
        a = (a * (n-1) + trs[i]) / n
    return a

def fmt(x):
    return f"{x:,.0f}" if x is not None else "n/a"

def brief(pair):
    dd, dm = fetch(pair, "1day", 300)
    hd, hm = fetch(pair, "1hour", 48)
    inc = bool(dm.get("lastIsIncomplete"))
    conf = dd[:-1] if inc else dd            # 確定足のみで指標
    closes = [r["close"] for r in conf]

    px = dd[-1]["close"]
    op = dd[-1]["open"]
    intraday = (px - op) / op * 100

    r = rsi(closes); m = macd(closes); a = atr(conf)
    s20, s50, s200 = sma(closes, 20), sma(closes, 50), sma(closes, 200)
    pos = []
    for lbl, s in (("20", s20), ("50", s50), ("200", s200)):
        if s: pos.append(("<" if px < s else ">") + "S" + lbl)
    below = sum(1 for s in (s20, s50, s200) if s and px < s)
    trend = "DOWN" if below >= 2 else ("UP" if below == 0 else "MIX")
    mh = m[2] if m else 0
    msign = "+" if mh > 0 else "-"

    # 曜日別出来高（確定30日）
    days = []
    for r0 in (conf[-30:]):
        dt = datetime.datetime.fromtimestamp(r0["timestamp"] / 1000, JST)
        days.append((dt.weekday(), r0["vol"]))
    def wavg(sel):
        xs = [v for w, v in days if sel(w)]
        return statistics.mean(xs) if xs else None
    wk = wavg(lambda w: w < 5); sat = wavg(lambda w: w == 5)
    sun = wavg(lambda w: w == 6); d30 = statistics.mean([v for _, v in days])

    # 本日の時間足合計（JST日付）
    now_dt = datetime.datetime.fromtimestamp(hd[-1]["timestamp"] / 1000, JST)
    today = now_dt.date()
    tvol = sum(x["vol"] for x in hd
               if datetime.datetime.fromtimestamp(x["timestamp"]/1000, JST).date() == today)

    # 直近確定日（例: 金曜）の値動き
    last = conf[-1]
    lastdt = datetime.datetime.fromtimestamp(last["timestamp"] / 1000, JST)
    lchg = (last["close"] - last["open"]) / last["open"] * 100

    satpct = f"{sat/wk*100:.0f}%" if (sat and wk) else "n/a"
    print(f"## {pair}  px={fmt(px)} ({intraday:+.1f}% intraday)  "
          f"RSI{r:.0f} MACD{msign} trend:{trend}[{' '.join(pos)}]"
          + ("  ※日足未確定" if inc else ""))
    print(f"   vol today(JST,so far)={tvol:,.1f} | wk={fmt(wk)} "
          f"sat={fmt(sat)}({satpct}) sun={fmt(sun)} 30d={fmt(d30)}")
    print(f"   {WD[lastdt.weekday()]} {lastdt:%m-%d}(確定): "
          f"{fmt(last['open'])}->{fmt(last['close'])} ({lchg:+.1f}%)  ATR14={fmt(a)}")

def main():
    pairs = sys.argv[1:] or os.environ.get(
        "BITBANK_BRIEF_PAIRS", "btc_jpy eth_jpy xrp_jpy").split()
    now = datetime.datetime.now(JST)
    print(f"# bitbank daily brief {now:%Y-%m-%d %a} JST {now:%H:%M}")
    for p in pairs:
        try:
            brief(p)
        except Exception as e:
            print(f"## {p}  ERROR: {e}")

if __name__ == "__main__":
    main()
