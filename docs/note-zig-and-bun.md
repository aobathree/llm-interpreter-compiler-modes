# 周辺メモ: 本プロジェクトにおける Zig の使われ方と、Bun の Rust 書き換えについて

日付: 2026-08-02

`aws-hourly-brief/` のビルドで Zig を使ったことをきっかけに出た
「Zigを使ったのか？ Zig製のBunはメモリーリークが多くRustに書き換えられたのでは？」
という問いへの整理。

## 本プロジェクトでの Zig：実装言語ではなくビルド道具

Lambda 用バイナリのビルドは `cargo-lambda → cargo-zigbuild → zig cc` という構成。
Zig ツールチェーンには clang と多数ターゲット向けの libc ヘッダーが同梱されており、
`zig cc` を**クロスコンパイル用の C コンパイラー／リンカー**として使うと、
Windows 上から Linux/arm64 向けの Rust バイナリを簡単にリンクできる。役割はそれだけ。

- 書いたコードは 100% Rust。Zig のソースコードは1行もない
- デプロイした `bootstrap` バイナリに Zig のランタイムは含まれず、
  メモリ安全性の特性は純粋に Rust のもの
- Bun とは状況が根本的に異なる。Bun は**アプリケーション本体が Zig で
  書かれていた**ケース、本プロジェクトは**リンカーとして Zig の
  ツールチェーンを借りた**ケース

導入は `pip install cargo-lambda ziglang`。Git Bash では ziglang 同梱の
`zig.exe` を PATH に通す必要がある（`aws-hourly-brief/README.md` 参照。
なお `export PATH="C:\...:$PATH"` は `C:` のコロンで PATH が分断されるので
`cygpath -u` で POSIX 形式に変換すること——今回実際に踏んだ罠）。

## Bun の Rust 書き換え（2026年、事実関係）

指摘はほぼ正確だった。Web 検索（2026-08-02 実施）による確認:

- 2026年5月11日、Bun 作者 Jarred Sumner が PR「Rewrite Bun in Rust」
  （約101万行・6,755コミット）を main にマージ、7月に公表
- 移植は **Claude エージェントの並列フリートを使って11日間**で実施
  （API 換算 約$165,000）。背景に 2025年12月の Anthropic による
  Bun（Oven社）買収がある
- ただし Rust 版は現時点で**実験的**（Linux x64 glibc のみ、テスト互換 99.8%）。
  安定版 1.3.x は依然 **Zig コードベース**から出荷されている
- Zig 作者 Andrew Kelley はこの移植を「レビューされていない粗製乱造
  (unreviewed slop)」と批判し、論争になっている
- 「メモリーリークが動機」と断定する公式記述は確認できなかった
  （Zig は手動メモリ管理でリーク系バグのリスクが構造的に高いのは事実で、
  Bun にもリーク報告は多かったが、公式の動機説明は Bun ブログを参照）

出典:
- https://bun.com/blog/bun-in-rust
- https://www.theregister.com/devops/2026/05/14/anthropics-bun-rust-rewrite-merged-at-speed-of-ai/5240381
- https://www.theregister.com/devops/2026/07/14/zig-creator-calls-buns-claude-rust-rewrite-unreviewed-slop/5270743
- https://en.wikipedia.org/wiki/Bun_(software)

## 本プロジェクトの文脈での含意

- ビルド道具としての Zig（zig cc）の利用は、Bun の件とは独立に今後も妥当
  （クロスリンクの手軽さは代替が少ない）
- 皮肉な符合として、Bun の Rust 移植は Claude のエージェント群が行い、
  本リポジトリの Rust 実装群も Claude（Claude Code）が書いた
- 「実装言語としての Zig vs Rust」の議論と「ビルドツールチェーンとしての
  Zig 利用」は切り分けて考えるべき、というのが本メモの結論
