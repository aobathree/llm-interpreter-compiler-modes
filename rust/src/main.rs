//! bitbank daily brief — tools/daily_brief.py の Rust 移植（コンパイル・モードの実証）。
//!
//! 出力フォーマットは Python 版と同一。違いは実行モデルのみ:
//! - ネイティブバイナリ（インタープリター起動なし）
//! - fetch（ペア × 時間枠）を並列実行（同時数上限は既定16、
//!   BITBANK_BRIEF_CONCURRENCY で変更可）
//!
//! 使い方:
//!   daily_brief                     # 既定ペア(btc_jpy eth_jpy xrp_jpy)
//!   daily_brief btc_jpy eth_jpy     # ペア指定
//!   BITBANK_BRIEF_PAIRS="btc_jpy sol_jpy" daily_brief
//!
//! 依存: `bitbank` CLI が PATH にあること。

use chrono::{Datelike, FixedOffset, TimeZone, Utc};
use serde_json::Value;
use std::process::Command;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Mutex, OnceLock};

const WD: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/// 同時 fetch 数の上限。銘柄数が多いとき、無制限の並列バーストは
/// API レート制限を踏んで大きな遅延を招く（benchmark/results.md 参照）。
/// 既定 16。環境変数 BITBANK_BRIEF_CONCURRENCY で変更可。
struct Semaphore {
    tx: Sender<()>,
    rx: Mutex<Receiver<()>>,
}
struct Permit<'a>(&'a Semaphore);
impl Semaphore {
    fn new(n: usize) -> Self {
        let (tx, rx) = channel();
        for _ in 0..n {
            tx.send(()).unwrap();
        }
        Semaphore { tx, rx: Mutex::new(rx) }
    }
    fn acquire(&self) -> Permit<'_> {
        self.rx.lock().unwrap().recv().expect("semaphore closed");
        Permit(self)
    }
}
impl Drop for Permit<'_> {
    fn drop(&mut self) {
        let _ = self.0.tx.send(());
    }
}

fn fetch_semaphore() -> &'static Semaphore {
    static S: OnceLock<Semaphore> = OnceLock::new();
    S.get_or_init(|| {
        let n = std::env::var("BITBANK_BRIEF_CONCURRENCY")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .filter(|&n| n >= 1)
            .unwrap_or(16);
        Semaphore::new(n)
    })
}

fn jst() -> FixedOffset {
    FixedOffset::east_opt(9 * 3600).unwrap()
}

struct Candle {
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    vol: f64,
    ts_ms: i64,
}

fn num(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

fn fetch(pair: &str, tf: &str, limit: u32) -> Result<(Vec<Candle>, bool), String> {
    let _permit = fetch_semaphore().acquire();
    let out = Command::new("bitbank")
        .args([
            "candles",
            pair,
            &format!("--type={tf}"),
            &format!("--limit={limit}"),
            "--format=json",
            "--machine",
        ])
        .output()
        .map_err(|e| format!("{pair} {tf}: {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let msg = if stderr.trim().is_empty() { stdout.trim() } else { stderr.trim() };
        return Err(format!("{pair} {tf}: {msg}"));
    }
    let j: Value =
        serde_json::from_str(&stdout).map_err(|e| format!("{pair} {tf}: bad json: {e}"))?;
    if j["success"].as_bool() != Some(true) {
        return Err(format!("{pair} {tf}: envelope success=false"));
    }
    let rows = j["data"]
        .as_array()
        .ok_or_else(|| format!("{pair} {tf}: data is not an array"))?;
    let mut candles = Vec::with_capacity(rows.len());
    for r in rows {
        candles.push(Candle {
            open: num(&r["open"]).ok_or_else(|| format!("{pair} {tf}: bad open"))?,
            high: num(&r["high"]).ok_or_else(|| format!("{pair} {tf}: bad high"))?,
            low: num(&r["low"]).ok_or_else(|| format!("{pair} {tf}: bad low"))?,
            close: num(&r["close"]).ok_or_else(|| format!("{pair} {tf}: bad close"))?,
            vol: num(&r["vol"]).ok_or_else(|| format!("{pair} {tf}: bad vol"))?,
            ts_ms: num(&r["timestamp"]).ok_or_else(|| format!("{pair} {tf}: bad timestamp"))?
                as i64,
        });
    }
    let incomplete = j["meta"]["lastIsIncomplete"].as_bool().unwrap_or(false);
    Ok((candles, incomplete))
}

fn sma(v: &[f64], n: usize) -> Option<f64> {
    (v.len() >= n).then(|| v[v.len() - n..].iter().sum::<f64>() / n as f64)
}

fn ema(v: &[f64], n: usize) -> Vec<f64> {
    let k = 2.0 / (n as f64 + 1.0);
    let mut e = v[0];
    let mut out = vec![e];
    for &x in &v[1..] {
        e = x * k + e * (1.0 - k);
        out.push(e);
    }
    out
}

fn rsi(c: &[f64], n: usize) -> Option<f64> {
    if c.len() < n + 1 {
        return None;
    }
    let g: Vec<f64> = c.windows(2).map(|w| (w[1] - w[0]).max(0.0)).collect();
    let l: Vec<f64> = c.windows(2).map(|w| (w[0] - w[1]).max(0.0)).collect();
    let mut ag = g[..n].iter().sum::<f64>() / n as f64;
    let mut al = l[..n].iter().sum::<f64>() / n as f64;
    for i in n..g.len() {
        ag = (ag * (n as f64 - 1.0) + g[i]) / n as f64;
        al = (al * (n as f64 - 1.0) + l[i]) / n as f64;
    }
    Some(if al == 0.0 { 100.0 } else { 100.0 - 100.0 / (1.0 + ag / al) })
}

fn macd_hist(c: &[f64]) -> Option<f64> {
    if c.len() < 35 {
        return None;
    }
    let e12 = ema(c, 12);
    let e26 = ema(c, 26);
    let line: Vec<f64> = e12.iter().zip(&e26).map(|(a, b)| a - b).collect();
    let sig = ema(&line[25..], 9);
    Some(line[line.len() - 1] - sig[sig.len() - 1])
}

fn atr(rows: &[Candle], n: usize) -> Option<f64> {
    if rows.len() < n + 1 {
        return None;
    }
    let trs: Vec<f64> = (1..rows.len())
        .map(|i| {
            let (h, l, pc) = (rows[i].high, rows[i].low, rows[i - 1].close);
            (h - l).max((h - pc).abs()).max((l - pc).abs())
        })
        .collect();
    let mut a = trs[..n].iter().sum::<f64>() / n as f64;
    for i in n..trs.len() {
        a = (a * (n as f64 - 1.0) + trs[i]) / n as f64;
    }
    Some(a)
}

/// Python の f"{x:,.0f}" 相当（3桁カンマ区切り）。None は "n/a"。
fn fmt(x: Option<f64>) -> String {
    match x {
        Some(v) => commas(v, 0),
        None => "n/a".into(),
    }
}

fn commas(x: f64, dec: usize) -> String {
    let s = format!("{:.*}", dec, x);
    let (sign, rest) = match s.strip_prefix('-') {
        Some(r) => ("-", r),
        None => ("", s.as_str()),
    };
    let (int, frac) = match rest.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (rest, None),
    };
    let mut out = String::from(sign);
    let len = int.len();
    for (i, ch) in int.chars().enumerate() {
        if i > 0 && (len - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    if let Some(f) = frac {
        out.push('.');
        out.push_str(f);
    }
    out
}

fn brief(pair: &str) -> Result<String, String> {
    // 日足と時足を並列 fetch
    let (dres, hres) = std::thread::scope(|s| {
        let h = s.spawn(|| fetch(pair, "1hour", 48));
        let d = fetch(pair, "1day", 300);
        (d, h.join().expect("hourly fetch thread panicked"))
    });
    let (dd, inc) = dres?;
    let (hd, _) = hres?;
    if dd.is_empty() || hd.is_empty() {
        return Err(format!("{pair}: empty data"));
    }

    let conf = if inc { &dd[..dd.len() - 1] } else { &dd[..] }; // 確定足のみで指標
    let closes: Vec<f64> = conf.iter().map(|r| r.close).collect();

    let px = dd[dd.len() - 1].close;
    let op = dd[dd.len() - 1].open;
    let intraday = (px - op) / op * 100.0;

    let r = rsi(&closes, 14).ok_or_else(|| format!("{pair}: not enough data for RSI"))?;
    let mh = macd_hist(&closes).unwrap_or(0.0);
    let a = atr(conf, 14);
    let (s20, s50, s200) = (sma(&closes, 20), sma(&closes, 50), sma(&closes, 200));
    let mut pos = Vec::new();
    for (lbl, s) in [("20", s20), ("50", s50), ("200", s200)] {
        if let Some(s) = s {
            pos.push(format!("{}S{lbl}", if px < s { "<" } else { ">" }));
        }
    }
    let below = [s20, s50, s200]
        .iter()
        .filter(|s| matches!(s, Some(v) if px < *v))
        .count();
    let trend = if below >= 2 { "DOWN" } else if below == 0 { "UP" } else { "MIX" };
    let msign = if mh > 0.0 { "+" } else { "-" };

    // 曜日別出来高（確定30日）
    let tz = jst();
    let start = conf.len().saturating_sub(30);
    let days: Vec<(u32, f64)> = conf[start..]
        .iter()
        .map(|r| {
            let wd = tz
                .timestamp_millis_opt(r.ts_ms)
                .unwrap()
                .weekday()
                .num_days_from_monday();
            (wd, r.vol)
        })
        .collect();
    let wavg = |sel: &dyn Fn(u32) -> bool| -> Option<f64> {
        let xs: Vec<f64> = days.iter().filter(|(w, _)| sel(*w)).map(|(_, v)| *v).collect();
        (!xs.is_empty()).then(|| xs.iter().sum::<f64>() / xs.len() as f64)
    };
    let wk = wavg(&|w| w < 5);
    let sat = wavg(&|w| w == 5);
    let sun = wavg(&|w| w == 6);
    let d30 = days.iter().map(|(_, v)| v).sum::<f64>() / days.len() as f64;

    // 本日の時間足合計（JST日付）
    let today = tz
        .timestamp_millis_opt(hd[hd.len() - 1].ts_ms)
        .unwrap()
        .date_naive();
    let tvol: f64 = hd
        .iter()
        .filter(|x| tz.timestamp_millis_opt(x.ts_ms).unwrap().date_naive() == today)
        .map(|x| x.vol)
        .sum();

    // 直近確定日の値動き
    let last = &conf[conf.len() - 1];
    let lastdt = tz.timestamp_millis_opt(last.ts_ms).unwrap();
    let lchg = (last.close - last.open) / last.open * 100.0;

    let satpct = match (sat, wk) {
        (Some(s), Some(w)) if w != 0.0 => format!("{:.0}%", s / w * 100.0),
        _ => "n/a".into(),
    };
    let inc_note = if inc { "  ※日足未確定" } else { "" };
    Ok(format!(
        "## {pair}  px={} ({intraday:+.1}% intraday)  RSI{r:.0} MACD{msign} trend:{trend}[{}]{inc_note}\n\
         \x20  vol today(JST,so far)={} | wk={} sat={}({satpct}) sun={} 30d={}\n\
         \x20  {} {}(確定): {}->{} ({lchg:+.1}%)  ATR14={}",
        fmt(Some(px)),
        pos.join(" "),
        commas(tvol, 1),
        fmt(wk),
        fmt(sat),
        fmt(sun),
        fmt(Some(d30)),
        WD[lastdt.weekday().num_days_from_monday() as usize],
        lastdt.format("%m-%d"),
        fmt(Some(last.open)),
        fmt(Some(last.close)),
        fmt(a),
    ))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let pairs: Vec<String> = if !args.is_empty() {
        args
    } else {
        std::env::var("BITBANK_BRIEF_PAIRS")
            .unwrap_or_else(|_| "btc_jpy eth_jpy xrp_jpy".into())
            .split_whitespace()
            .map(String::from)
            .collect()
    };
    let now = Utc::now().with_timezone(&jst());
    println!(
        "# bitbank daily brief {} JST {}",
        now.format("%Y-%m-%d %a"),
        now.format("%H:%M")
    );
    // 全ペアを並列処理し、指定順に出力
    let results: Vec<String> = std::thread::scope(|s| {
        let handles: Vec<_> = pairs
            .iter()
            .map(|p| {
                s.spawn(move || brief(p).unwrap_or_else(|e| format!("## {p}  ERROR: {e}")))
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("pair thread panicked"))
            .collect()
    });
    for r in results {
        println!("{r}");
    }
}
