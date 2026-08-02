//! lambda-config-api — 配信設定の GET/PUT（Function URL で公開）。
//!
//! - 認証: `X-Auth-Token` ヘッダーと SSM SecureString の共有トークンを比較
//! - GET  /: 現在の設定 + 選択可能な銘柄一覧（売買代金降順）を返す
//! - PUT  /: 設定を検証して DynamoDB に保存（次回の brief 実行から反映）
//!
//! 環境変数: TABLE_NAME, TOKEN_PARAM, ALLOWED_ORIGIN（既定 "*"）

use aws_sdk_dynamodb::types::AttributeValue;
use lambda_http::{run, service_fn, Body, Error, Request, Response};
use serde_json::{json, Value};
use tokio::sync::OnceCell;

static TOKEN: OnceCell<String> = OnceCell::const_new();

async fn expected_token() -> Result<&'static String, Error> {
    TOKEN
        .get_or_try_init(|| async {
            let name = std::env::var("TOKEN_PARAM")?;
            let conf = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
            let ssm = aws_sdk_ssm::Client::new(&conf);
            let p = ssm.get_parameter().name(&name).with_decryption(true).send().await?;
            let v = p
                .parameter()
                .and_then(|p| p.value())
                .ok_or("token parameter has no value")?;
            Ok::<String, Error>(v.to_string())
        })
        .await
}

fn respond(status: u16, body: Value) -> Result<Response<Body>, Error> {
    let origin = std::env::var("ALLOWED_ORIGIN").unwrap_or_else(|_| "*".into());
    Ok(Response::builder()
        .status(status)
        .header("Content-Type", "application/json; charset=utf-8")
        .header("Access-Control-Allow-Origin", origin)
        .header("Access-Control-Allow-Methods", "GET, PUT, OPTIONS")
        .header("Access-Control-Allow-Headers", "Content-Type, X-Auth-Token")
        .body(Body::from(body.to_string()))?)
}

async fn db() -> Result<(aws_sdk_dynamodb::Client, String), Error> {
    let table = std::env::var("TABLE_NAME")?;
    let conf = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
    Ok((aws_sdk_dynamodb::Client::new(&conf), table))
}

async fn get_config_item() -> Result<Value, Error> {
    let (client, table) = db().await?;
    let item = client
        .get_item()
        .table_name(&table)
        .key("pk", AttributeValue::S("config".into()))
        .send()
        .await?
        .item;
    let Some(item) = item else {
        return Ok(json!({
            "mode": "pairs",
            "pairs": ["btc_jpy", "eth_jpy", "xrp_jpy"],
            "top_n": 10,
            "enabled": true,
            "delivery": "chunks",
            "note": "(default — no record yet)"
        }));
    };
    let s = |k: &str| item.get(k).and_then(|v| v.as_s().ok().cloned());
    let pairs: Vec<String> = item
        .get("pairs")
        .and_then(|v| v.as_l().ok())
        .map(|l| l.iter().filter_map(|v| v.as_s().ok().cloned()).collect())
        .unwrap_or_default();
    Ok(json!({
        "mode": s("mode").unwrap_or_else(|| "pairs".into()),
        "pairs": pairs,
        "top_n": item.get("top_n").and_then(|v| v.as_n().ok()).and_then(|n| n.parse::<u64>().ok()).unwrap_or(10),
        "enabled": item.get("enabled").and_then(|v| v.as_bool().ok().copied()).unwrap_or(true),
        "delivery": s("delivery").unwrap_or_else(|| "chunks".into()),
        "updated_at": s("updated_at").unwrap_or_default(),
        "note": s("note").unwrap_or_default(),
    }))
}

async fn put_config(body: &Value, available: &[String]) -> Result<Value, Error> {
    let mode = body["mode"].as_str().unwrap_or("pairs");
    if !["pairs", "top", "all"].contains(&mode) {
        return Ok(json!({ "error": "mode must be pairs | top | all" }));
    }
    let pairs: Vec<String> = body["pairs"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();
    if mode == "pairs" {
        if pairs.is_empty() {
            return Ok(json!({ "error": "mode=pairs requires a non-empty pairs list" }));
        }
        for p in &pairs {
            if !available.contains(p) {
                return Ok(json!({ "error": format!("unknown pair: {p}") }));
            }
        }
    }
    let top_n = body["top_n"].as_u64().unwrap_or(10).clamp(1, 100);
    let enabled = body["enabled"].as_bool().unwrap_or(true);
    let delivery = match body["delivery"].as_str().unwrap_or("chunks") {
        "file" => "file",
        _ => "chunks",
    };
    let note = body["note"].as_str().unwrap_or("").chars().take(200).collect::<String>();
    let updated_at = chrono::Utc::now()
        .with_timezone(&chrono::FixedOffset::east_opt(9 * 3600).unwrap())
        .to_rfc3339();

    let (client, table) = db().await?;
    client
        .put_item()
        .table_name(&table)
        .item("pk", AttributeValue::S("config".into()))
        .item("mode", AttributeValue::S(mode.into()))
        .item(
            "pairs",
            AttributeValue::L(pairs.iter().map(|p| AttributeValue::S(p.clone())).collect()),
        )
        .item("top_n", AttributeValue::N(top_n.to_string()))
        .item("enabled", AttributeValue::Bool(enabled))
        .item("delivery", AttributeValue::S(delivery.into()))
        .item("note", AttributeValue::S(note))
        .item("updated_at", AttributeValue::S(updated_at))
        .send()
        .await?;
    get_config_item().await
}

async fn handler(req: Request) -> Result<Response<Body>, Error> {
    let method = req.method().as_str().to_string();
    if method == "OPTIONS" {
        return respond(204, json!({}));
    }
    let token = req
        .headers()
        .get("x-auth-token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if token.is_empty() || token != expected_token().await?.as_str() {
        return respond(401, json!({ "error": "invalid token" }));
    }

    // 選択可能銘柄は bitbank tickers から（GUI が bitbank と直接通信しないための代理取得）
    let available = tokio::task::spawn_blocking(brief_core::ranked_pairs)
        .await?
        .unwrap_or_default();

    match method.as_str() {
        "GET" => {
            let config = get_config_item().await?;
            respond(200, json!({ "config": config, "available": available }))
        }
        "PUT" => {
            let body: Value = match req.body() {
                Body::Text(t) => serde_json::from_str(t).unwrap_or(Value::Null),
                Body::Binary(b) => serde_json::from_slice(b).unwrap_or(Value::Null),
                _ => Value::Null,
            };
            if body.is_null() {
                return respond(400, json!({ "error": "invalid JSON body" }));
            }
            let result = put_config(&body, &available).await?;
            if result.get("error").is_some() {
                respond(400, result)
            } else {
                respond(200, json!({ "config": result, "available": available }))
            }
        }
        _ => respond(405, json!({ "error": "method not allowed" })),
    }
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    run(service_fn(handler)).await
}
