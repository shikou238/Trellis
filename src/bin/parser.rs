//! DSL パーサー単体の動作確認用。
//! `cargo run --bin parser [ファイル]` でファイルを、引数なしなら下のサンプルをパースして AST を表示する。

use learn_wgpu2::parser;

const SAMPLE: &str = r#"
        (defparam s 100 (Int 0 100 _))
        (defparam t 0 (Float 0 _ 0.16))
        (defparam i0 "test-ui.png" (Image))

        (background (rgb 100 200 0))

        (paint rect {s+t*100} {s + t * 100} 200 300)
        (paint i0 {s + t * 50 + 100} {s + t * 50 + 100} 200 300)

        ()

        (defvar m1 (matrix 3 3 (( 1 2 0 ) ( 4 5 0 ) ( 7 8 1 ))) (Matrix33))
        (paint rect {s+t*100} {s + t * 100} 200 300 m1)

        (log s)
        (log t)
"#;

fn main() {
    let source = match std::env::args().nth(1) {
        Some(path) => std::fs::read_to_string(&path).expect("failed to read file"),
        None => SAMPLE.to_string(),
    };

    match parser::parse(&source) {
        Ok(ast) => println!("{:#?}", ast),
        Err(e) => {
            let (line, col) = parser::line_col(&source, e.span.start);
            eprintln!("parse error at {}:{}: {}", line, col, e.message);
            std::process::exit(1);
        }
    }
}
