# aws-hourly-brief

bitbank の商い状況ダイジェストを1時間ごとに Discord へ配信する
AWS サーバーレス実装。設計は [DESIGN.md](DESIGN.md) を参照。

```
EventBridge rate(1 hour) → Lambda (Rust/arm64) → bitbank public API → Discord Webhook
GUI (gui/index.html) → config-api Lambda (Function URL) → DynamoDB（配信銘柄を動的変更）
```

## 構成

| パス | 内容 |
|---|---|
| [brief-core/](brief-core/) | ダイジェスト生成の共有ライブラリ（`rust-direct/` から抽出。出力同一） |
| [lambda-brief/](lambda-brief/) | 毎時実行の本体（設定読取 → 生成 → Discord送信） |
| [lambda-config-api/](lambda-config-api/) | 設定 GET/PUT API（トークン認証・CORS対応） |
| [gui/index.html](gui/index.html) | 銘柄選択ページ（静的1枚。手元で開くだけで使える） |
| [template.yaml](template.yaml) | SAM テンプレート（Lambda×2・DynamoDB・スケジュール・ログ保持7日） |

## デプロイ手順

### 0. 前提

- AWS CLI（認証設定済み）と SAM CLI
- cargo-lambda（`pip install cargo-lambda ziglang` が手軽。cargo install でも可）

Windows + Git Bash で pip の ziglang を使う場合、zig.exe を PATH に通す:

```bash
export PATH="$(cygpath -u "$(python -c 'import ziglang,os;print(os.path.dirname(ziglang.__file__))')"):$PATH"
```

### 1. シークレットを SSM に登録（初回のみ）

```bash
aws ssm put-parameter --name /hourly-brief/discord-webhook --type SecureString --value "https://discord.com/api/webhooks/..."
```

```bash
aws ssm put-parameter --name /hourly-brief/config-token --type SecureString --value "任意の長いランダム文字列"
```

### 2. ビルドとデプロイ

```bash
cd aws-hourly-brief
make build
make deploy-guided   # 初回。2回目以降は make deploy
```

デプロイ完了時の出力 `ConfigApiUrl`（Function URL）を控える。

### 3. 動作確認

```bash
make invoke   # 手動実行 → Discord にダイジェストが届けば成功
```

以後は毎時自動実行される。既定の配信銘柄は btc_jpy / eth_jpy / xrp_jpy
（DynamoDB に設定レコードが無い場合のフォールバック）。

### 4. 銘柄の動的変更

[gui/index.html](gui/index.html) をブラウザーで開き、`ConfigApiUrl` とトークンを
入力して「読み込み」→ 銘柄を選び直して「保存」。**次回の配信から反映**される
（デプロイ不要）。GUI を使わない場合は AWS CLI でも同じことができる:

```bash
aws dynamodb put-item --table-name <ConfigTableName> --item '{"pk":{"S":"config"},"mode":{"S":"top"},"top_n":{"N":"10"},"enabled":{"BOOL":true},"pairs":{"L":[]},"delivery":{"S":"chunks"}}'
```

## 設定レコードの意味

| 属性 | 値 | 説明 |
|---|---|---|
| mode | `pairs` / `top` / `all` | 銘柄指定 / 売買代金上位N / 全銘柄 |
| pairs | 文字列リスト | mode=pairs のときの対象 |
| top_n | 数値 | mode=top のときの銘柄数 |
| enabled | bool | false で配信を一時停止 |
| delivery | `chunks` / `file` | 2000字制限への対応: 分割送信 / .txt 添付 |

## ローカル確認（AWS 不要）

```bash
cargo run -p brief-core --example print -- btc_jpy eth_jpy
```

`rust-direct/` 版と同一入力・同時刻で実行すれば、銘柄セクションは
1文字単位で一致する（ゴールデン確認）。
