# aws-hourly-brief — build & deploy
#
# 前提: cargo-lambda（pip install cargo-lambda ziglang でも可）, SAM CLI, AWS 認証設定
#
#   make build    # arm64 向けに 2 つの Lambda を zip ビルド
#   make deploy   # sam deploy（初回は make deploy-guided）
#   make invoke   # デプロイ済み brief Lambda を手動実行

STACK ?= aws-hourly-brief

build:
	cargo lambda build --release --arm64 --output-format zip

deploy: build
	sam deploy --stack-name $(STACK) --resolve-s3 --capabilities CAPABILITY_IAM --no-confirm-changeset

deploy-guided: build
	sam deploy --guided --stack-name $(STACK) --capabilities CAPABILITY_IAM

invoke:
	aws lambda invoke --function-name aws-hourly-brief --payload '{}' /dev/stdout

.PHONY: build deploy deploy-guided invoke
