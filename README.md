# LLMのインタープリター・モードとコンパイル・モード

LLM活用の二相モデル——初回は LLM による試行錯誤で手順を発見し（**インタープリター・モード**）、
2回目以降は確立した手順をスクリプトに固定化して高速・低トークンで再実行する（**コンパイル・モード**）——
の考察記事と、その実証コードです。

## 内容

| ファイル | 説明 |
|---|---|
| [article-llm-interpreter-compiler-modes.md](article-llm-interpreter-compiler-modes.md) | 本編記事（図解入り・用語注釈付き） |
| [docs/article-llm-interpreter-compiler-modes.pdf](docs/article-llm-interpreter-compiler-modes.pdf) | 記事のPDF版（A4・2ページ。印刷原稿は同フォルダーの article-print-source.html） |
| [llm-interpreter-compile-mode-ChatGPT.md](llm-interpreter-compile-mode-ChatGPT.md) | ChatGPT との対話ドラフト（概念の体系化） |
| [llm-interpreter-compile-mode-Cursor.md](llm-interpreter-compile-mode-Cursor.md) | Cursor との対話ドラフト（業界潮流・設計課題） |
| [tools/daily_brief.py](tools/daily_brief.py) | 実証コード：暗号通貨の日次商い状況を約0.4秒で集計するスクリプト |
| [tools/daily_brief.md](tools/daily_brief.md) | daily_brief.py の設計意図と運用ノート |

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
