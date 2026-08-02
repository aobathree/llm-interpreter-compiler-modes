#!/usr/bin/env python3
"""rust/（CLIサブプロセス版）と rust-direct/（REST API直接版）の比較ベンチマーク。

売買代金上位から 3 / 10 / 30 / 全銘柄 を選び、両実装の実行時間を交互に計測する。
出力: 標準出力 + benchmark/results-direct.md

使い方:
  python benchmark/compare_direct.py            # 各条件3回計測
  python benchmark/compare_direct.py --runs 5 --cooldown 8
"""
import argparse, datetime, json, pathlib, statistics, subprocess, sys, time

ROOT = pathlib.Path(__file__).resolve().parent.parent
BINS = {
    "CLI版 (rust/)": ROOT / "rust/target/release/daily_brief.exe",
    "直接API版 (rust-direct/)": ROOT / "rust-direct/target/release/daily_brief_direct.exe",
}

def tickers():
    out = subprocess.run(["bitbank", "tickers", "--format=json", "--machine"],
                         capture_output=True, text=True, encoding="utf-8")
    if out.returncode != 0:
        sys.exit(f"bitbank tickers failed: {out.stderr.strip()}")
    return json.loads(out.stdout)["data"]

def rank_by_jpy_volume(rows):
    btc_jpy = next((float(r["last"]) for r in rows if r["pair"] == "btc_jpy"), None)
    ranked = []
    for r in rows:
        pair, vol, last = r["pair"], float(r["vol"]), float(r["last"])
        notional = vol * last
        if pair.endswith("_btc"):
            if btc_jpy is None:
                continue
            notional *= btc_jpy
        elif not pair.endswith("_jpy"):
            continue
        ranked.append((pair, notional))
    ranked.sort(key=lambda x: -x[1])
    return [p for p, _ in ranked]

def run_once(binpath, pairs):
    t0 = time.perf_counter()
    out = subprocess.run([str(binpath)] + pairs, capture_output=True, text=True,
                         encoding="utf-8")
    dt = time.perf_counter() - t0
    errors = sum(1 for l in out.stdout.splitlines() if "ERROR" in l)
    return dt, errors, out.returncode

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--runs", type=int, default=3)
    ap.add_argument("--cooldown", type=float, default=8.0)
    args = ap.parse_args()
    for name, b in BINS.items():
        if not b.exists():
            sys.exit(f"binary not found: {b} — build it first (cargo build --release)")

    pairs_all = rank_by_jpy_volume(tickers())
    sizes = [3, 10, 30, len(pairs_all)]
    results = {name: [] for name in BINS}

    for n in sizes:
        pairs = pairs_all[:n]
        for name, binpath in BINS.items():
            dt, ne, rc = run_once(binpath, pairs)   # warmup（統計除外）
            print(f"  top{n} {name}: warmup = {dt:.2f}s")
            time.sleep(args.cooldown)
            times, errs = [], []
            for r in range(args.runs):
                dt, ne, rc = run_once(binpath, pairs)
                times.append(dt); errs.append(ne)
                print(f"  top{n} {name}: run{r+1} = {dt:.2f}s (errors={ne}, rc={rc})")
                time.sleep(args.cooldown)
            results[name].append((n, statistics.median(times), min(times), max(times), max(errs)))

    lines = ["# CLI版 vs REST API直接版 ベンチマーク", "",
             f"実施日: {datetime.date.today().isoformat()} / 各条件 {args.runs} 回計測"
             f"（+ウォームアップ1回、実行間クールダウン{args.cooldown:.0f}秒）",
             "両実装とも同時fetch上限16。出力は1文字単位で同一。", "",
             "| 銘柄数 | CLI版 中央値（最小〜最大） | 直接API版 中央値（最小〜最大） | 短縮 |",
             "|---|---|---|---|"]
    for i, n in enumerate(sizes):
        _, cm, cmin, cmax, _ = results["CLI版 (rust/)"][i]
        _, dm, dmin, dmax, _ = results["直接API版 (rust-direct/)"][i]
        speedup = f"{cm/dm:.1f}x" if dm > 0 else "n/a"
        lines.append(f"| {n} | {cm:.2f}s（{cmin:.2f}〜{cmax:.2f}） "
                     f"| {dm:.2f}s（{dmin:.2f}〜{dmax:.2f}） | {speedup} |")
    report = "\n".join(lines) + "\n"
    (ROOT / "benchmark" / "results-direct.md").write_text(report, encoding="utf-8")
    print()
    print(report)

if __name__ == "__main__":
    main()
