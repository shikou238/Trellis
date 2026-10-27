//! メニュー試作: 方式A・方式B の 2 つのシーンとインスペクタ。T キーで計測モード。

mod analysis;
mod menu;
mod session;

fn main() {
    // ui/*.ui と logs/ はこのクレートからの相対パスで読み書きする（どこから cargo run しても同じ）
    std::env::set_current_dir(env!("CARGO_MANIFEST_DIR")).expect("could not enter the crate directory");
    engine::run(menu::window_specs(), menu::MenuProto::new).unwrap();
}
