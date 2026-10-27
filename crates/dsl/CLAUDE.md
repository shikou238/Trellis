# dsl

DSL の構文（S式 + {中置式}）のレキサーとパーサー。設計は [DESIGN.md](DESIGN.md)、書き方の一覧は [../uikit/DSL.md](../uikit/DSL.md)。
**wgpu・winit・OS を知らない**。依存クレートを足さない。

## 規約
- ソース上の位置（`Span`、char インデックス）をすべてのノードとエラーに持たせる。インスペクタの書き戻しとエラー表示に使う
- 非決定的な操作（現在時刻など）を入れない
- 構文を変えたら uikit の `DSL.md` と、このクレートの `DESIGN.md` §2 を直し、`cargo test -p uikit -p menu-proto` を回す（シーンとログの読み込みがこのパーサーを使う）

## 実行
```bash
cargo run -p dsl --bin parser [ファイル]   # AST を表示
```
