//! lambda-brief — 毎時実行される本体。
//!
//! EventBridge から起動され、
//! 1. DynamoDB から配信設定を読む（失敗時は既定3銘柄にフォールバック）
//! 2. brief-core でダイジェスト生成（並列fetch・同時数上限16）
//! 3. Discord Webhook へ送信（2000字制限に合わせペア境界でチャンク分割）
//!
//! 環境変数: TABLE_NAME, WEBHOOK_PARAM（SSM SecureString名）,
//!           BITBANK_BRIEF_CONCURRENCY（既定16）

use aws_sdk_dynamodb::types::AttributeValue;
use lambda_runtime::{service_fn, Error, LambdaEvent};
use serde_json::Value;
use tokio::sync::OnceCell;

const DEFAULT_PAIRS: [&str; 3] = ["btc_jpy", "eth_jpy", "xrp_jpy"];
/// Discord の本文上限2000字。コードブロック記法と余白を差し引いた実効上限。
const CHUNK_LIMIT: usize = 1900;

#[derive(Debug, Clone)]
struct BriefConfig {
    mode: String,      // "pairs" | "top" | "all"
    pairs: Vec<String>,
    top_n: usize,
    enabled: bool,
    delivery: String,  // "chunks" | "file"
}

impl Default for BriefConfig {
    fn default() -> Self {
        BriefConfig {
            mode: "pairs".into(),
            pairs: DEFAULT_PAIRS.iter().map(|s| s.to_string()).collect(),
            top_n: 10,
            enabled: true,
            delivery: "chunks".into(),
        }
    }
}

static WEBHOOK_URL: OnceCell<String> = OnceCell::const_new();

async fn webhook_url() -> Result<&'static String, Error> {
    WEBHOOK_URL
        .get_or_try_init(|| async {
            let name = std::env::var("WEBHOOK_PARAM")?;
            let conf = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
            let ssm = aws_sdk_ssm::Client::new(&conf);
            let p = ssm
                .get_parameter()
                .name(&name)
                .with_decryption(true)
                .send()
                .await?;
            let v = p
                .parameter()
                .and_then(|p| p.value())
                .ok_or("webhook parameter has no value")?;
            Ok::<String, Error>(v.to_string())
        })
        .await
}

async fn read_config() -> BriefConfig {
    let result = async {
        let table = std::env::var("TABLE_NAME")?;
        let conf = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
        let db = aws_sdk_dynamodb::Client::new(&conf);
        let item = db
            .get_item()
            .table_name(&table)
            .key("pk", AttributeValue::S("config".into()))
            .send()
            .await?
            .item
            .ok_or("config item not found")?;
        let mut c = BriefConfig::default();
        if let Some(AttributeValue::S(m)) = item.get("mode") {
            c.mode = m.clone();
        }
        if let Some(AttributeValue::L(l)) = item.get("pairs") {
            let pairs: Vec<String> = l
                .iter()
                .filter_map(|v| match v {
                    AttributeValue::S(s) => Some(s.clone()),
                    _ => None,
                })
                .collect();
            if !pairs.is_empty() {
                c.pairs = pairs;
            }
        }
        if let Some(AttributeValue::N(n)) = item.get("top_n") {
            if let Ok(n) = n.parse() {
                c.top_n = n;
            }
        }
        if let Some(AttributeValue::Bool(b)) = item.get("enabled") {
            c.enabled = *b;
        }
        if let Some(AttributeValue::S(d)) = item.get("delivery") {
            c.delivery = d.clone();
        }
        Ok::<BriefConfig, Error>(c)
    }
    .await;
    match result {
        Ok(c) => c,
        Err(e) => {
            // 設定が読めなくても配信は止めない（設計3.1）
            println!("config read failed, using defaults: {e}");
            BriefConfig::default()
        }
    }
}

async fn resolve_pairs(cfg: &BriefConfig) -> Result<Vec<String>, Error> {
    match cfg.mode.as_str() {
        "top" | "all" => {
            let n = if cfg.mode == "top" { cfg.top_n } else { usize::MAX };
            let ranked = tokio::task::spawn_blocking(brief_core::ranked_pairs)
                .await?
                .map_err(Error::from)?;
            Ok(ranked.into_iter().take(n).collect())
        }
        _ => Ok(cfg.pairs.clone()),
    }
}

/// セクションをペア境界で CHUNK_LIMIT 以下のメッセージ群にまとめる。
fn chunk_messages(header: &str, sections: &[String]) -> Vec<String> {
    let mut msgs = Vec::new();
    let mut cur = header.to_string();
    for s in sections {
        if !cur.is_empty() && cur.len() + s.len() + 1 > CHUNK_LIMIT {
            msgs.push(cur);
            cur = String::new();
        }
        if !cur.is_empty() {
            cur.push('\n');
        }
        cur.push_str(s);
    }
    if !cur.is_empty() {
        msgs.push(cur);
    }
    msgs
}

fn post_discord_json(url: &str, content: &str) -> Result<(), String> {
    let body = serde_json::json!({ "content": format!("```text\n{content}\n```") });
    for attempt in 0..2 {
        match ureq::post(url).send_json(&body) {
            Ok(_) => return Ok(()),
            Err(ureq::Error::Status(429, resp)) if attempt == 0 => {
                let wait = resp
                    .into_json::<Value>()
                    .ok()
                    .and_then(|j| j["retry_after"].as_f64())
                    .unwrap_or(1.0);
                std::thread::sleep(std::time::Duration::from_secs_f64(wait.min(10.0)));
            }
            Err(e) => return Err(format!("discord post failed: {e}")),
        }
    }
    Err("discord post failed after retry".into())
}

fn post_discord_file(url: &str, summary: &str, filename: &str, text: &str) -> Result<(), String> {
    let boundary = "----aws-hourly-brief-boundary";
    let payload = serde_json::json!({ "content": summary }).to_string();
    let body = format!(
        "--{boundary}\r\n\
         Content-Disposition: form-data; name=\"payload_json\"\r\n\
         Content-Type: application/json\r\n\r\n{payload}\r\n\
         --{boundary}\r\n\
         Content-Disposition: form-data; name=\"files[0]\"; filename=\"{filename}\"\r\n\
         Content-Type: text/plain; charset=utf-8\r\n\r\n{text}\r\n\
         --{boundary}--\r\n"
    );
    ureq::post(url)
        .set("Content-Type", &format!("multipart/form-data; boundary={boundary}"))
        .send_bytes(body.as_bytes())
        .map(|_| ())
        .map_err(|e| format!("discord file post failed: {e}"))
}

fn send_to_discord(url: &str, cfg: &BriefConfig, header: &str, sections: &[String]) -> Result<usize, String> {
    if cfg.delivery == "file" {
        let text = format!("{header}\n{}", sections.join("\n"));
        post_discord_file(url, header, "brief.txt", &text)?;
        return Ok(1);
    }
    let msgs = chunk_messages(header, sections);
    let n = msgs.len();
    for (i, m) in msgs.iter().enumerate() {
        post_discord_json(url, m)?;
        if i + 1 < n {
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
    }
    Ok(n)
}

async fn handler(_event: LambdaEvent<Value>) -> Result<Value, Error> {
    let cfg = read_config().await;
    println!("config: {cfg:?}");
    if !cfg.enabled {
        println!("disabled by config; skipping");
        return Ok(serde_json::json!({ "skipped": true }));
    }
    let pairs = resolve_pairs(&cfg).await?;
    let t0 = std::time::Instant::now();
    let (header, sections) =
        tokio::task::spawn_blocking(move || brief_core::brief_many(&pairs)).await?;
    let elapsed = t0.elapsed().as_secs_f64();
    let errors = sections.iter().filter(|s| s.contains("ERROR")).count();
    println!("brief generated: {} sections, {} errors, {:.2}s", sections.len(), errors, elapsed);
    println!("{header}\n{}", sections.join("\n"));

    let url = webhook_url().await?.clone();
    let cfg2 = cfg.clone();
    let sent = tokio::task::spawn_blocking(move || {
        if sections.is_empty() || errors == sections.len() {
            // 全滅時も沈黙させず1行だけ通知（設計3.3）
            post_discord_json(&url, &format!("{header}\n(all pairs failed: {errors} errors)"))
                .map(|_| 1)
        } else {
            send_to_discord(&url, &cfg2, &header, &sections)
        }
    })
    .await?
    .map_err(Error::from)?;
    println!("discord: {sent} message(s) sent");
    Ok(serde_json::json!({ "messages": sent, "errors": errors, "elapsed_s": elapsed }))
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    lambda_runtime::run(service_fn(handler)).await
}
