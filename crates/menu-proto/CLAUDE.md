# menu-proto

メニュー試作（方式A・方式B・インスペクタ・計測モード）。UI ツール（uikit）自体を試す場所。設計は [DESIGN.md](DESIGN.md)、`ui/*.ui` の書き方は [../uikit/DSL.md](../uikit/DSL.md)。
ゲームと作曲ソフトを知らない。

## 実行
```bash
cargo run -p menu-proto    # どこから実行してもよい（起動時にこのクレートのディレクトリに移る）
cargo test -p menu-proto   # ui/*.ui を変えたときも回す（同梱のシーンを読み込むテストがある）
```
操作: Space/矢印/Enter/Backspace=UI（キー配置は DESIGN.md §4）, R=ホットリロード, T=計測の開始/中断, Esc=終了
起動すると「方式A」「方式B」「インスペクタ」の3ウィンドウが開く。`ui/*.ui` は保存すると自動で反映される

## 規約
- 計測の判定はシーンの中身を見ず、シーンの `emit` の通知だけで行う
- ログの形式を変えたら `analysis.rs` の読み込みと DESIGN.md §3 の例も直す。ログは集計の唯一の材料なので、記録できる情報は捨てない
- 乱数はシード付き（SplitMix64）。シードはログに残す
