# uikit

UI シーン DSL の実行（`ScenePlayer`, `Scene`）とパラメータのインスペクタ。設計は [DESIGN.md](DESIGN.md)、書き方の一覧は [DSL.md](DSL.md)。
**個々の製品を知らない**。依存は dsl（構文）と engine（描画・入力・ファイル）だけ。

## 規約
- 定義（`Scene`）はリロードで差し替え、実行時の状態（`Runtime`, tick）は保持する。失敗したら直前の正常版で動き続ける
- 名前はコンパイル時にスロット番号へ解決する。毎フレーム名前で引かない。DSL の評価はなるべくリロード時に 1 回
- 時間は固定タイムステップの tick 数から求める（決定性）。DSL に非決定的な操作を入れない
- DSL の書き方を変えたら [DSL.md](DSL.md) も直す
- 同梱のシーン（menu-proto の `ui/*.ui`）を読むテストは menu-proto にある。DSL を変えたら `cargo test -p uikit -p menu-proto`
