//! ゴールデン確認用: brief-core の出力を標準出力に出す。
//! rust-direct 版と同一入力で diff することで抽出の等価性を確認する。
//!
//!   cargo run -p brief-core --example print -- btc_jpy eth_jpy

fn main() {
    let pairs: Vec<String> = std::env::args().skip(1).collect();
    let pairs = if pairs.is_empty() {
        vec!["btc_jpy".into(), "eth_jpy".into(), "xrp_jpy".into()]
    } else {
        pairs
    };
    let (header, sections) = brief_core::brief_many(&pairs);
    println!("{header}");
    for s in sections {
        println!("{s}");
    }
}
