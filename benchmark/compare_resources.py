#!/usr/bin/env python3
"""CLI版 vs REST API直接版のリソース消費比較（Windows Job Object 使用）。

壁時計時間だけでなく、プロセスツリー全体（親＋全子プロセス）の
- CPU時間（user + kernel）
- 総プロセス数
- ピークメモリ（ジョブ全体 / 単一プロセス最大）
- I/O転送量
を Job Object のアカウンティングで正確に計測する。
ポーリング方式と異なり、短命の子プロセス（CLI版は88個起動する）も取りこぼさない。

使い方:
  python benchmark/compare_resources.py            # 3銘柄と44銘柄で各3回
  python benchmark/compare_resources.py --runs 5

出力: 標準出力 + benchmark/results-resources.md
依存: Windows / 標準ライブラリのみ
"""
import argparse, ctypes, datetime, json, pathlib, statistics, subprocess, sys, time
from ctypes import wintypes as wt

ROOT = pathlib.Path(__file__).resolve().parent.parent
BINS = {
    "CLI版": ROOT / "rust/target/release/daily_brief.exe",
    "直接API版": ROOT / "rust-direct/target/release/daily_brief_direct.exe",
}

k32 = ctypes.windll.kernel32

class IO_COUNTERS(ctypes.Structure):
    _fields_ = [(n, ctypes.c_ulonglong) for n in (
        "ReadOperationCount", "WriteOperationCount", "OtherOperationCount",
        "ReadTransferCount", "WriteTransferCount", "OtherTransferCount")]

class BASIC_LIMIT(ctypes.Structure):
    _fields_ = [("PerProcessUserTimeLimit", ctypes.c_longlong),
                ("PerJobUserTimeLimit", ctypes.c_longlong),
                ("LimitFlags", wt.DWORD),
                ("MinimumWorkingSetSize", ctypes.c_size_t),
                ("MaximumWorkingSetSize", ctypes.c_size_t),
                ("ActiveProcessLimit", wt.DWORD),
                ("Affinity", ctypes.c_size_t),
                ("PriorityClass", wt.DWORD),
                ("SchedulingClass", wt.DWORD)]

class EXTENDED_LIMIT(ctypes.Structure):
    _fields_ = [("BasicLimitInformation", BASIC_LIMIT),
                ("IoInfo", IO_COUNTERS),
                ("ProcessMemoryLimit", ctypes.c_size_t),
                ("JobMemoryLimit", ctypes.c_size_t),
                ("PeakProcessMemoryUsed", ctypes.c_size_t),
                ("PeakJobMemoryUsed", ctypes.c_size_t)]

class BASIC_ACCOUNTING(ctypes.Structure):
    _fields_ = [("TotalUserTime", ctypes.c_longlong),
                ("TotalKernelTime", ctypes.c_longlong),
                ("ThisPeriodTotalUserTime", ctypes.c_longlong),
                ("ThisPeriodTotalKernelTime", ctypes.c_longlong),
                ("TotalPageFaultCount", wt.DWORD),
                ("TotalProcesses", wt.DWORD),
                ("ActiveProcesses", wt.DWORD),
                ("TotalTerminatedProcesses", wt.DWORD)]

class BASIC_AND_IO(ctypes.Structure):
    _fields_ = [("BasicInfo", BASIC_ACCOUNTING), ("IoInfo", IO_COUNTERS)]

JobObjectBasicAndIoAccountingInformation = 8
JobObjectExtendedLimitInformation = 9

def run_in_job(binpath, pairs):
    """コマンドを Job Object 内で実行し、ツリー全体のリソース使用量を返す。"""
    job = k32.CreateJobObjectW(None, None)
    if not job:
        sys.exit("CreateJobObject failed")
    t0 = time.perf_counter()
    p = subprocess.Popen([str(binpath)] + pairs,
                         stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    # 直後に割り当て。以降に生成される子プロセスは自動的にジョブへ入る
    if not k32.AssignProcessToJobObject(job, int(p._handle)):
        p.kill()
        sys.exit("AssignProcessToJobObject failed")
    rc = p.wait()
    wall = time.perf_counter() - t0

    acct = BASIC_AND_IO()
    k32.QueryInformationJobObject(job, JobObjectBasicAndIoAccountingInformation,
                                  ctypes.byref(acct), ctypes.sizeof(acct), None)
    ext = EXTENDED_LIMIT()
    k32.QueryInformationJobObject(job, JobObjectExtendedLimitInformation,
                                  ctypes.byref(ext), ctypes.sizeof(ext), None)
    k32.CloseHandle(job)
    return {
        "rc": rc,
        "wall_s": wall,
        "cpu_user_s": acct.BasicInfo.TotalUserTime / 1e7,     # 100ns 単位
        "cpu_kernel_s": acct.BasicInfo.TotalKernelTime / 1e7,
        "processes": acct.BasicInfo.TotalProcesses,
        "page_faults": acct.BasicInfo.TotalPageFaultCount,
        "peak_job_mem_mb": ext.PeakJobMemoryUsed / 2**20,
        "peak_proc_mem_mb": ext.PeakProcessMemoryUsed / 2**20,
        "io_read_mb": acct.IoInfo.ReadTransferCount / 2**20,
        "io_write_mb": acct.IoInfo.WriteTransferCount / 2**20,
        "io_other_mb": acct.IoInfo.OtherTransferCount / 2**20,
    }

# 旧ティッカー（現行取扱い44銘柄には含まれない）
LEGACY_PAIRS = {"matic_jpy", "rndr_jpy", "mkr_jpy"}

def tickers_ranked():
    """公式取扱い銘柄（44種類・JPY建て）の24h売買代金降順ランキング。"""
    out = subprocess.run(["bitbank", "tickers", "--format=json", "--machine"],
                         capture_output=True, text=True, encoding="utf-8")
    rows = json.loads(out.stdout)["data"]
    ranked = []
    for r in rows:
        pair = r["pair"]
        if not pair.endswith("_jpy") or pair in LEGACY_PAIRS:
            continue
        ranked.append((pair, float(r["vol"]) * float(r["last"])))
    ranked.sort(key=lambda x: -x[1])
    return [p for p, _ in ranked]

def med(rows, key):
    return statistics.median(r[key] for r in rows)

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--runs", type=int, default=3)
    ap.add_argument("--cooldown", type=float, default=8.0)
    args = ap.parse_args()
    for name, b in BINS.items():
        if not b.exists():
            sys.exit(f"binary not found: {b}")

    all_pairs = tickers_ranked()
    sizes = [3, len(all_pairs)]
    results = {}   # (size, name) -> list of dicts

    for n in sizes:
        pairs = all_pairs[:n]
        for name, binpath in BINS.items():
            r = run_in_job(binpath, pairs)   # warmup（統計除外）
            print(f"  top{n} {name}: warmup wall={r['wall_s']:.2f}s")
            time.sleep(args.cooldown)
            rows = []
            for i in range(args.runs):
                r = run_in_job(binpath, pairs)
                rows.append(r)
                print(f"  top{n} {name}: run{i+1} wall={r['wall_s']:.2f}s "
                      f"cpu={r['cpu_user_s']+r['cpu_kernel_s']:.2f}s "
                      f"procs={r['processes']} peakJobMem={r['peak_job_mem_mb']:.0f}MB "
                      f"(rc={r['rc']})")
                time.sleep(args.cooldown)
            results[(n, name)] = rows

    lines = ["# CLI版 vs REST API直接版 リソース消費比較", "",
             f"実施日: {datetime.date.today().isoformat()} / 各条件 {args.runs} 回計測の中央値"
             f"（+ウォームアップ1回、実行間クールダウン{args.cooldown:.0f}秒）",
             "計測方法: Windows Job Object によるプロセスツリー全体のアカウンティング",
             "（短命の子プロセスも取りこぼさない）", ""]
    for n in sizes:
        lines += [f"## {n}銘柄", "",
                  "| 指標 | CLI版 | 直接API版 | 比 |",
                  "|---|---|---|---|"]
        a = results[(n, "CLI版")]
        b = results[(n, "直接API版")]
        def ratio_str(va, vb):
            return f"{va / vb:.1f}x" if vb else "-"
        def row(label, key, fmt):
            va, vb = med(a, key), med(b, key)
            lines.append(f"| {label} | {fmt.format(va)} | {fmt.format(vb)} | {ratio_str(va, vb)} |")
        row("壁時計時間", "wall_s", "{:.2f}秒")
        acpu = med(a, "cpu_user_s") + med(a, "cpu_kernel_s")
        bcpu = med(b, "cpu_user_s") + med(b, "cpu_kernel_s")
        lines.append(f"| CPU時間合計 (user+kernel) | {acpu:.2f}秒 | {bcpu:.2f}秒 | {ratio_str(acpu, bcpu)} |")
        lines.append(f"| 　うち user | {med(a,'cpu_user_s'):.2f}秒 | {med(b,'cpu_user_s'):.2f}秒 | |")
        lines.append(f"| 　うち kernel | {med(a,'cpu_kernel_s'):.2f}秒 | {med(b,'cpu_kernel_s'):.2f}秒 | |")
        row("総プロセス数", "processes", "{:.0f}")
        row("ピークメモリ（ジョブ全体）", "peak_job_mem_mb", "{:.1f}MB")
        row("ピークメモリ（単一プロセス最大）", "peak_proc_mem_mb", "{:.1f}MB")
        row("ページフォルト", "page_faults", "{:,.0f}")
        row("I/O read 転送量", "io_read_mb", "{:.1f}MB")
        row("I/O other 転送量（ソケット含む）", "io_other_mb", "{:.1f}MB")
        lines.append("")
    report = "\n".join(lines) + "\n"
    (ROOT / "benchmark" / "results-resources.md").write_text(report, encoding="utf-8")
    print()
    print(report)

if __name__ == "__main__":
    main()
