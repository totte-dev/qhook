# Private タスク一覧

OSS公開しないタスク。Cloud版、インフラ、ビジネス関連。

## v0.3 開発基盤強化 ✅ 完了

| タスク | 状態 | 備考 |
|--------|------|------|
| SECURITY.md | 完了 | 脆弱性報告の窓口 |
| CONTRIBUTING.md | 完了 | 開発ガイド・TDD方針の明文化 |
| リリース自動化ワークフロー | 完了 | workflow_dispatch: GitHub Release + crates.io + Docker Hub |
| OpenAPI spec | 完了 | 手書き OpenAPI 3.1、redocly lint通過 |
| cargo-audit CI統合 | 完了 | ci.ymlにcargo audit追加 |
| エラーコードドキュメント | 完了 | docs/guides/error-reference.md |
| API versioning | 完了 | X-API-Version: 1 レスポンスヘッダー |
| SDK自動生成 (Python/Go/TS) | 完了 | sdks/generate.sh スキャフォールド。パッケージ公開は未実施 |
| k6ベンチCI統合 | 完了 | tests/bench.js + 週次CI (bench.yml) |

---

## v0.3 Tier 1: コア品質強化（OSS価値向上）

市場で誰もやっていない or 弱い領域。セルフホスト価値を高める。

| タスク | 優先度 | 状態 | 備考 |
|--------|--------|------|------|
| サーキットブレーカー | High | 完了 | handler単位。連続失敗N回で一時停止→自動復帰。競合に無い差別化 |
| OpenTelemetry対応 | High | 完了 | optional feature `otel`。OTLP export + tracing統合 |
| イベントリプレイ強化 | Medium | 完了 | `events replay` CLI。--source/--event-type/--since/--until フィルタ |
| Helmチャート | Medium | 完了 | charts/qhook/。Deployment/Service/ConfigMap/Ingress/PVC/HPA |
| ホットリロード | Low | 完了 | SIGHUP で差分ログ出力（要restart項目も警告） |

## v0.3 Tier 2: 差別化機能（市場ギャップ）

qhookの独自性を強化する新機能。

| タスク | 優先度 | 状態 | 備考 |
|--------|--------|------|------|
| Outbound webhooks | High | 未着手 | SaaS→顧客へのwebhook送信。Svix $490/mo相当。署名付与、リトライ、顧客別エンドポイント管理 |
| 高度なフィルタ演算子 | Medium | 未着手 | `contains`, `starts_with`, `regex`, `exists`, `not` |
| イベントスキーマ検証 | Medium | 未着手 | JSON Schema validation。不正ペイロードを受信時にreject |
| バッチ配信 | Low | 未着手 | 同一エンドポイントへの複数イベント一括送信。IoT/高スループット向け |
| Kafka/SQS入力ソース | Low | 未着手 | メッセージブローカーからの消費。Convoyが対応済み |

## v0.3 Tier 3: Cloud版基盤

有料Cloud版の中核機能。OSS版には含まない。

| タスク | 優先度 | 状態 | 備考 |
|--------|--------|------|------|
| Web UIダッシュボード | High | 未着手 | イベント一覧、ジョブ状態、リプレイ、メトリクス可視化。Cloud版の中核価値 |
| マルチテナント設計 | High | 未着手 | DB分離方式の決定。テナント別API Key/レートリミット |
| SSO/SAML | Medium | 未着手 | 企業向け必須。$500+/mo tier相当 |
| 監査ログ | Medium | 未着手 | 全操作の記録。コンプライアンス（金融/ヘルスケア） |
| 分散レートリミット (Redis) | Medium | 未着手 | マルチインスタンス時のグローバルレートリミット |
| IPアローリスト / ブロックリスト | Low | 未着手 | テナント別のIP制御 |

---

## インフラ / ドメイン

| タスク | 優先度 | 状態 |
|--------|--------|------|
| ドメイン取得 (qhook.dev / qhook.io 等) | High | 未着手 |
| DNS設定 + GitHub Pages カスタムドメイン | High | ドメイン取得後 |
| crates.io publish | High | v0.2.2 公開済 |
| Docker Hub / GHCR イメージ公開 | Medium | release.yml整備済。シークレット設定要 |
| awesome-rust PR | Medium | publish後 |

## マーケティング / 認知

| タスク | 優先度 | 状態 |
|--------|--------|------|
| 技術記事: qhook紹介（日本語 Zenn/Qiita） | High | 下書き済 (`article-ja.md`) |
| 技術記事: qhook紹介（英語 dev.to/Medium） | High | 下書き済 (`article-en.md`) |
| 技術記事: Claude CodeでOSS開発（日本語） | High | 下書き済 (`article-claude-dev.md`) |
| GitHub repo Topics設定 | Medium | 未着手 |
| Social preview画像 (OGP) | Medium | 未着手 |
| awesome-webhooks PR | Low | 未着手 |

## 課金 / ビジネス

| タスク | 優先度 | 状態 | 備考 |
|--------|--------|------|------|
| 料金プラン設計 | High | 未着手 | Free / Starter $20/mo / Pro $99/mo / Enterprise。市場の$490ギャップを狙う |
| Stripe Billing連携 | High | 未着手 | |
| ランディングページ | High | ドメイン取得後 | |
| 利用規約 / プライバシーポリシー | Medium | 未着手 | |

---

## 戦略メモ

### qhookのポジショニング
**「ゼロ依存・単一バイナリの唯一のwebhookゲートウェイ」**

- Convoy/Svix → Postgres+Redis必須。qhookはSQLiteで動く
- Svix → Outboundのみ。qhookはIn/Out両方
- 全競合 → gRPC出力なし、cronなし。qhookは両方ある

### 市場の価格ギャップ
- Svix: Free → $10/mo → **$490/mo** (Professional)
- Hookdeck: Free → $39/mo → $499/mo
- qhook Cloud: Free → **$20/mo** → $99/mo で中間を取る

### 競合にない差別化（実装済み or 予定）
1. 単一バイナリ・ゼロ依存 ✅
2. gRPC出力 ✅
3. 内蔵cron ✅
4. サーキットブレーカー ✅
5. Outbound webhooks（予定）
