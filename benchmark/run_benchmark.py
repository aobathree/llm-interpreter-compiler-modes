#!/usr/bin/env python3
"""daily_brief (Rust版) のスケーリング・ベンチマーク。

bitbank 全銘柄を24時間売買代金（JPY換算）の多い順に並べ、
トップ10 / 20 / 30 / 全銘柄で Rust 版 daily_brief の実行時間を計測する。

使い方:
  python benchmark/run_benchmark.py            # 各条件3回計測
  python benchmark/run_benchmark.py --runs 5

前提: `bitbank` CLI と rust/target/release/daily_brief(.exe) がビルド済み。
出力: 標準出力 + benchmark/results.md
"""
import argparse, datetime, json, pathlib, statistics, subprocess, sys, time

ROOT = pathlib.Path(__file__).resolve().parent.parent
BIN = next((p for p in [ROOT / "rust/target/release/daily_brief.exe",
                        ROOT / "rust/target/release/daily_brief"] if p.exists()), None)

def tickers():
    out = subprocess.run(["bitbank", "tickers", "--format=json", "--machine"],
                         capture_output=True, text=True, encoding="utf-8")
    if out.returncode != 0:
        sys.exit(f"bitbank tickers failed: {out.stderr.strip()}")
    j = json.loads(out.stdout)
    return j["data"]

# 旧ティッカーのペア。tickers API には残っているが、bitbank の現行取扱い
# 銘柄（44種類）には含まれない（MATIC→POL・RNDR→RENDER 移行後、MKR は取扱い外）。
LEGACY_PAIRS = {"matic_jpy", "rndr_jpy", "mkr_jpy"}

def rank_by_jpy_volume(rows):
    """公式取扱い銘柄（44種類・JPY建て）の24h売買代金降順ランキング。
    tickers API に載る旧ティッカー・BTC建てクロスペアは取扱い銘柄ではないため除外。"""
    ranked = []
    for r in rows:
        pair = r["pair"]
        if not pair.endswith("_jpy") or pair in LEGACY_PAIRS:
            continue
        ranked.append((pair, float(r["vol"]) * float(r["last"])))
    ranked.sort(key=lambda x: -x[1])
    return ranked

def run_once(pairs):
    t0 = time.perf_counter()
    out = subprocess.run([str(BIN)] + pairs, capture_output=True, text=True,
                         encoding="utf-8")
    dt = time.perf_counter() - t0
    errors = [l for l in out.stdout.splitlines() if "ERROR" in l]
    return dt, len(errors), out.returncode

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--runs", type=int, default=5)
    ap.add_argument("--warmup", type=int, default=1,
                    help="統計から除外する事前実行の回数（接続キャッシュ等を暖める）")
    ap.add_argument("--cooldown", type=float, default=10.0,
                    help="実行間の待機秒数（APIレート制限の回復を待つ）")
    args = ap.parse_args()
    if BIN is None:
        sys.exit("rust binary not found — run: cd rust && cargo build --release")

    rows = tickers()
    ranked = rank_by_jpy_volume(rows)
    print(f"# pairs ranked by 24h JPY notional volume ({len(ranked)} pairs)")
    for i, (p, v) in enumerate(ranked[:10], 1):
        print(f"  {i:2d}. {p:14s} {v:,.0f} JPY")
    print("  ...")

    sizes = [10, 20, 30, len(ranked)]
    results = []
    for n in sizes:
        pairs = [p for p, _ in ranked[:n]]
        for w in range(args.warmup):
            dt, ne, rc = run_once(pairs)
            print(f"  top{n}: warmup = {dt:.2f}s  (errors={ne}, rc={rc})")
            time.sleep(args.cooldown)
        times, errs = [], []
        for r in range(args.runs):
            dt, ne, rc = run_once(pairs)
            times.append(dt); errs.append(ne)
            print(f"  top{n}: run{r+1} = {dt:.2f}s  (errors={ne}, rc={rc})")
            time.sleep(args.cooldown)
        results.append((n, statistics.median(times), min(times), max(times), max(errs)))

    # レポート
    lines = ["# daily_brief (Rust) スケーリング・ベンチマーク", "",
             f"実施日: {datetime.date.today().isoformat()} / 各条件 {args.runs} 回計測",
             f"対象: bitbank 全 {len(ranked)} 銘柄を24時間売買代金（JPY換算）降順で選抜",
             f"fetch回数 = 銘柄数 × 2（日足300本 + 時足48本）を全並列実行", "",
             "| 銘柄数 | fetch回数 | 中央値 | 最小 | 最大 | ERROR行 |",
             "|---|---|---|---|---|---|"]
    for n, med, mn, mx, ne in results:
        lines.append(f"| {n} | {n*2} | {med:.2f}s | {mn:.2f}s | {mx:.2f}s | {ne} |")
    lines += ["", "## 売買代金ランキング（上位30）", "",
              "| 順位 | ペア | 24h売買代金(JPY) |", "|---|---|---|"]
    for i, (p, v) in enumerate(ranked[:30], 1):
        lines.append(f"| {i} | {p} | {v:,.0f} |")
    report = "\n".join(lines) + "\n"
    out_path = ROOT / "benchmark" / "results.md"
    out_path.write_text(report, encoding="utf-8")
    print(f"\nwrote {out_path}")
    for l in lines[5:5+len(sizes)+2]:
        print(l)

if __name__ == "__main__":
    main()
