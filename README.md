# LLMのインタープリター・モードとコンパイル・モード

LLM活用の二相モデル——初回は LLM による試行錯誤で手順を発見し（**インタープリター・モード**）、
2回目以降は確立した手順をスクリプトに固定化して高速・低トークンで再実行する（**コンパイル・モード**）——
の考察記事と、その実証コードです。

## 内容

| ファイル | 説明 |
|---|---|
| [article-llm-interpreter-compiler-modes.md](article-llm-interpreter-compiler-modes.md) | 本編記事（図解入り・用語注釈付き。8/2改訂: Rust実験を追記） |
| [docs/article-llm-interpreter-compiler-modes-20260802.pdf](docs/article-llm-interpreter-compiler-modes-20260802.pdf) | **改訂版PDF**（8/2付け・A4・5ページ。Rust実験〜本家への提案まで） |
| [docs/article-llm-interpreter-compiler-modes.pdf](docs/article-llm-interpreter-compiler-modes.pdf) | 初版PDF（8/1付け・A4・2ページ） |
| [llm-interpreter-compile-mode-ChatGPT.md](llm-interpreter-compile-mode-ChatGPT.md) | ChatGPT との対話ドラフト（概念の体系化） |
| [llm-interpreter-compile-mode-Cursor.md](llm-interpreter-compile-mode-Cursor.md) | Cursor との対話ドラフト（業界潮流・設計課題） |
| [tools/daily_brief.py](tools/daily_brief.py) | 実証コード：暗号通貨の日次商い状況を集計するスクリプト（Python） |
| [tools/daily_brief.md](tools/daily_brief.md) | daily_brief.py の設計意図と運用ノート |
| [rust/](rust/) | daily_brief の Rust 実装（`bitbank` CLI をサブプロセス起動・fetch並列化） |
| [rust-direct/](rust-direct/) | REST API 直接版（プロセス生成ゼロ・接続プーリング。出力は同一） |
| [benchmark/](benchmark/) | スケーリング実験＋CLI版 vs 直接API版の比較（レート制限の考察含む） |

## すぐ実験する

記事の実例「daily_brief.py」は、このリポジトリだけで動かせます。

### 前提

- Python 3.9 以上
- [bitbank-lab-cli](https://github.com/bitbankinc/bitbank-lab-cli) がインストール済みで、`bitbank` コマンドが PATH にあること
  （公開APIのローソク足取得のみ使用します。APIキーは不要です）

### 実行

```bash
python tools/daily_brief.py                  # 既定: btc_jpy eth_jpy xrp_jpy
python tools/daily_brief.py btc_jpy sol_jpy  # ペア指定
```

[uv](https://docs.astral.sh/uv/) ユーザーはこちらでも実行できます（依存パッケージは標準ライブラリのみなので、仮想環境の準備は不要です）。

```bash
uv run tools/daily_brief.py
```

### Rust版（さらに「コンパイル」を推し進めた実装）

出力フォーマットは Python 版と同一のまま、ネイティブバイナリ化し、
fetch（ペア × 時間枠 = 6回の CLI 呼び出し）をすべて並列実行します。

```bash
cd rust
cargo build --release
./target/release/daily_brief
```

実測（3ペア・中央値、Windows 11 / 単発の `bitbank candles` 呼び出し ≈ 0.1秒の環境）:

| 実装 | 実行時間 | 備考 |
|---|---|---|
| Python 版 | 約 0.6 秒 | 6回の fetch が逐次 |
| Rust 版 | **約 0.11 秒** | 6回の fetch を並列化 → 実質1回分の待ち時間 |

約5倍の高速化ですが、内訳は「Rustだから速い」ではなく、
①インタープリター起動が消えたこと、②fetch の並列化、が大半です。
計算部分（SMA/RSI/MACD/ATR）はどちらの実装でも数ミリ秒以下であり、
律速はネットワークです。コンパイル・モードの本当の価値は速度そのものより、
「手順が決定的なコードに固定されているからこそ、こうした並列化などの
古典的な最適化が安全に適用できる」点にあります。

銘柄数を増やしたときの挙動（全62銘柄で約1.2秒、無制限並列だとレート制限を踏む話）は
[benchmark/results.md](benchmark/results.md) を参照してください。同時 fetch 数は既定16で、
環境変数 `BITBANK_BRIEF_CONCURRENCY` で変更できます。

さらに `bitbank` CLI を経由せず REST API を直接呼ぶ [rust-direct/](rust-direct/) もあります
（出力は1文字単位で同一。62銘柄で約1.2倍・分散大幅減。詳細と考察は
[benchmark/results-direct.md](benchmark/results-direct.md)）。

```bash
./target/release/daily_brief btc_jpy eth_jpy sol_jpy doge_jpy  # 任意の銘柄数でOK
```

1ペアあたり3行の圧縮ダイジェストが出力されます。これを LLM に読ませれば、
生のローソク足JSON（数万トークン）を一切コンテキストに入れずに日次分析ができます。
出力の読み方は [tools/daily_brief.md](tools/daily_brief.md) を参照してください。

## 記事の要旨

1. **探索と実行の分離** — 未知の手順の発見は LLM（柔軟・高価）、発見済み手順の再実行はスクリプト（高速・安価・再現可能）
2. **三分解** — 固定できる処理はコードへ、可変値は入力変数へ、意味的判断だけ小さな LLM 呼び出しとして残す
3. **ループ** — 探索 → コンパイル → 高速実行 → 想定外だけ LLM へ復帰 → 再コンパイル（JIT の脱最適化に相当）
4. **実測効果** — LLM が読む量: 数万トークン → 10行弱 / 実行時間: 約0.44秒（2ペア）

## 著者

**aobathree**（ChatGPT・Claude・Cursor の助けを借りて作成）

## ライセンス

[MIT License](LICENSE)
