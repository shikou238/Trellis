//! UI シーン DSL のコンパイルと評価。
//!
//! リロード時に AST を `Scene` にコンパイルし、シンボルはスロット番号に解決しておく。
//! 毎フレームは `env`（f32 の配列）を引くだけで、名前による検索はしない。
//!
//! ```lisp
//! (defparam item_w 260 (Float 100 400 _))   ;; 名前 既定値 (型 最小 最大 刻み)  `_` は指定なし
//! (def top {height / 2 - 120})              ;; 毎フレーム計算される名前付きの値。使うより前の行で定義する
//! (background (rgb 24 24 32))
//! (rect x y w h (rgb 255 0 0) (rotate 15))  ;; 数値には リテラル / シンボル / {中置式} が書ける
//! ```
//! 組み込みの値: `time`（秒）, `width`, `height`（論理ピクセル）, `pi`
//! 関数: sin cos abs floor min max clamp lerp

use crate::parser::{self, AstKind, AstNode, BinaryOp, Expr, ParseError, ParseResult, Span, UnaryOp};
use crate::ui::renderer::RectInstance;

const SLOT_TIME: usize = 0;
const SLOT_WIDTH: usize = 1;
const SLOT_HEIGHT: usize = 2;
const BUILTIN_NAMES: [&str; 3] = ["time", "width", "height"];

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ParamKind {
    Int,
    Float,
}

#[derive(Debug, Clone)]
pub struct Param {
    pub name: String,
    pub kind: ParamKind,
    pub value: f32,
    pub min: Option<f32>,
    pub max: Option<f32>,
    pub step: Option<f32>,
    /// 既定値リテラルの位置。スライダーで変えた値を書き戻すときに使う
    pub value_span: Span,
}

pub struct Scene {
    pub params: Vec<Param>,
    /// `def` の式。スロットはパラメータの後ろに定義順で並ぶ
    defs: Vec<CExpr>,
    background: Option<ColorExpr>,
    rects: Vec<RectItem>,
}

struct RectItem {
    x: CExpr,
    y: CExpr,
    w: CExpr,
    h: CExpr,
    /// 度
    rotation: CExpr,
    color: ColorExpr,
}

/// 0–255 の sRGB 値と 0–255 のアルファ
struct ColorExpr([CExpr; 4]);

/// スロット解決済みの式
enum CExpr {
    Const(f32),
    Slot(usize),
    Neg(Box<CExpr>),
    Not(Box<CExpr>),
    Binary(BinaryOp, Box<CExpr>, Box<CExpr>),
    Call(Func, Vec<CExpr>),
}

#[derive(Clone, Copy)]
enum Func {
    Sin,
    Cos,
    Abs,
    Floor,
    Min,
    Max,
    Clamp,
    Lerp,
}

impl Func {
    fn lookup(name: &str) -> Option<(Func, usize)> {
        Some(match name {
            "sin" => (Func::Sin, 1),
            "cos" => (Func::Cos, 1),
            "abs" => (Func::Abs, 1),
            "floor" => (Func::Floor, 1),
            "min" => (Func::Min, 2),
            "max" => (Func::Max, 2),
            "clamp" => (Func::Clamp, 3),
            "lerp" => (Func::Lerp, 3),
            _ => return None,
        })
    }
}

impl CExpr {
    fn eval(&self, env: &[f32]) -> f32 {
        match self {
            CExpr::Const(v) => *v,
            CExpr::Slot(i) => env[*i],
            CExpr::Neg(e) => -e.eval(env),
            CExpr::Not(e) => {
                if e.eval(env) == 0.0 { 1.0 } else { 0.0 }
            }
            CExpr::Binary(op, l, r) => {
                let (l, r) = (l.eval(env), r.eval(env));
                match op {
                    BinaryOp::Add => l + r,
                    BinaryOp::Sub => l - r,
                    BinaryOp::Mul => l * r,
                    BinaryOp::Div => l / r,
                }
            }
            CExpr::Call(func, args) => {
                let a = |i: usize| args[i].eval(env);
                match func {
                    Func::Sin => a(0).sin(),
                    Func::Cos => a(0).cos(),
                    Func::Abs => a(0).abs(),
                    Func::Floor => a(0).floor(),
                    Func::Min => a(0).min(a(1)),
                    Func::Max => a(0).max(a(1)),
                    Func::Clamp => a(0).clamp(a(1), a(2)),
                    Func::Lerp => a(0) + (a(1) - a(0)) * a(2),
                }
            }
        }
    }
}

impl ColorExpr {
    /// 線形色空間の RGBA（sRGB サーフェスに書くため）
    fn eval(&self, env: &[f32]) -> [f32; 4] {
        let c = |i: usize| (self.0[i].eval(env) / 255.0).clamp(0.0, 1.0);
        [srgb_to_linear(c(0)), srgb_to_linear(c(1)), srgb_to_linear(c(2)), c(3)]
    }
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

impl Scene {
    pub fn compile(source: &str) -> ParseResult<Scene> {
        let nodes = parser::parse(source)?;
        let mut compiler = Compiler {
            names: BUILTIN_NAMES.iter().map(|s| s.to_string()).collect(),
        };

        // 1周目: パラメータを集める（定義より前の行からも参照できるように）
        let mut params = Vec::new();
        for node in &nodes {
            if let AstKind::Form { head, args } = &node.kind {
                if head == "defparam" {
                    let param = compile_defparam(args, node)?;
                    if compiler.names.contains(&param.name) {
                        return Err(error(node, format!("`{}` is already defined", param.name)));
                    }
                    compiler.names.push(param.name.clone());
                    params.push(param);
                }
            }
        }

        // 2周目: def と描画要素（上から順に）
        let mut defs = Vec::new();
        let mut background = None;
        let mut rects = Vec::new();
        for node in &nodes {
            let AstKind::Form { head, args } = &node.kind else {
                return Err(error(node, "expected a form like `(rect ...)`"));
            };
            match head.as_str() {
                "defparam" => {}
                "def" => {
                    let [name, value] = args.as_slice() else {
                        return Err(error(node, "usage: (def name value)"));
                    };
                    let AstKind::Symbol(name) = &name.kind else {
                        return Err(error(name, "name must be a symbol"));
                    };
                    if compiler.names.contains(name) {
                        return Err(error(node, format!("`{}` is already defined", name)));
                    }
                    defs.push(compiler.number(value)?);
                    compiler.names.push(name.clone());
                }
                "background" => {
                    let [color] = args.as_slice() else {
                        return Err(error(node, "usage: (background color)"));
                    };
                    background = Some(compiler.color(color)?);
                }
                "rect" => rects.push(compiler.rect(node, args)?),
                other => return Err(error(node, format!("unknown form `{}`", other))),
            }
        }

        Ok(Scene { params, defs, background, rects })
    }

    /// 毎フレームの評価に使う環境を作る（組み込み値 + パラメータ値 + def の値）
    pub fn fill_env(&self, env: &mut Vec<f32>, time: f32, width: f32, height: f32) {
        env.clear();
        env.resize(BUILTIN_NAMES.len(), 0.0);
        env[SLOT_TIME] = time;
        env[SLOT_WIDTH] = width;
        env[SLOT_HEIGHT] = height;
        env.extend(self.params.iter().map(|p| p.value));
        for def in &self.defs {
            let value = def.eval(env);
            env.push(value);
        }
    }

    pub fn background(&self, env: &[f32]) -> Option<[f32; 4]> {
        self.background.as_ref().map(|c| c.eval(env))
    }

    pub fn build_rects(&self, env: &[f32], out: &mut Vec<RectInstance>) {
        out.extend(self.rects.iter().map(|r| RectInstance {
            position: [r.x.eval(env), r.y.eval(env)],
            size: [r.w.eval(env), r.h.eval(env)],
            color: r.color.eval(env),
            rotation: r.rotation.eval(env).to_radians(),
        }));
    }
}

fn error(node: &AstNode, message: impl Into<String>) -> ParseError {
    ParseError {
        message: message.into(),
        span: node.span.clone(),
    }
}

fn literal_number(node: &AstNode) -> Option<f32> {
    match node.kind {
        AstKind::Integer(v) => Some(v as f32),
        AstKind::Float(v) => Some(v as f32),
        _ => None,
    }
}

fn compile_defparam(args: &[AstNode], node: &AstNode) -> ParseResult<Param> {
    const USAGE: &str = "usage: (defparam name default (Int|Float min max step))";
    let [name, default, ty] = args else {
        return Err(error(node, USAGE));
    };
    let AstKind::Symbol(name) = &name.kind else {
        return Err(error(name, "parameter name must be a symbol"));
    };
    let Some(value) = literal_number(default) else {
        return Err(error(default, "default value must be a number literal"));
    };
    let AstKind::Form { head, args: ty_args } = &ty.kind else {
        return Err(error(ty, USAGE));
    };
    let kind = match head.as_str() {
        "Int" => ParamKind::Int,
        "Float" => ParamKind::Float,
        other => return Err(error(ty, format!("unsupported parameter type `{}`", other))),
    };
    if ty_args.len() > 3 {
        return Err(error(ty, USAGE));
    }
    // min max step。`_` または省略は指定なし
    let mut bounds = [None; 3];
    for (slot, arg) in bounds.iter_mut().zip(ty_args) {
        *slot = match &arg.kind {
            AstKind::Symbol(s) if s == "_" => None,
            _ => Some(literal_number(arg).ok_or_else(|| error(arg, "expected a number or `_`"))?),
        };
    }
    Ok(Param {
        name: name.clone(),
        kind,
        value,
        min: bounds[0],
        max: bounds[1],
        step: bounds[2],
        value_span: default.span.clone(),
    })
}

struct Compiler {
    /// スロット番号 → 名前
    names: Vec<String>,
}

impl Compiler {
    fn slot(&self, name: &str, node: &AstNode) -> ParseResult<CExpr> {
        if name == "pi" {
            return Ok(CExpr::Const(std::f32::consts::PI));
        }
        match self.names.iter().position(|n| n == name) {
            Some(i) => Ok(CExpr::Slot(i)),
            None => Err(error(node, format!("unknown name `{}`", name))),
        }
    }

    fn number(&self, node: &AstNode) -> ParseResult<CExpr> {
        match &node.kind {
            AstKind::Integer(v) => Ok(CExpr::Const(*v as f32)),
            AstKind::Float(v) => Ok(CExpr::Const(*v as f32)),
            AstKind::Symbol(name) => self.slot(name, node),
            AstKind::Expr(expr) => self.expr(expr, node),
            _ => Err(error(node, "expected a number, name, or {expression}")),
        }
    }

    /// Expr は位置を持たないので、エラーは囲んでいるノードの位置で報告する
    fn expr(&self, expr: &Expr, node: &AstNode) -> ParseResult<CExpr> {
        Ok(match expr {
            Expr::Integer(v) => CExpr::Const(*v as f32),
            Expr::Float(v) => CExpr::Const(*v as f32),
            Expr::Symbol(name) => self.slot(name, node)?,
            Expr::Unary { op, expr } => {
                let inner = Box::new(self.expr(expr, node)?);
                match op {
                    UnaryOp::Neg => CExpr::Neg(inner),
                    UnaryOp::Not => CExpr::Not(inner),
                }
            }
            Expr::Binary { op, left, right } => CExpr::Binary(
                *op,
                Box::new(self.expr(left, node)?),
                Box::new(self.expr(right, node)?),
            ),
            Expr::Call { callee, args } => {
                let Some((func, arity)) = Func::lookup(callee) else {
                    return Err(error(node, format!("unknown function `{}`", callee)));
                };
                if args.len() != arity {
                    return Err(error(
                        node,
                        format!("`{}` takes {} argument(s), got {}", callee, arity, args.len()),
                    ));
                }
                let args = args.iter().map(|a| self.expr(a, node)).collect::<ParseResult<_>>()?;
                CExpr::Call(func, args)
            }
        })
    }

    fn color(&self, node: &AstNode) -> ParseResult<ColorExpr> {
        const USAGE: &str = "expected (rgb r g b) or (rgba r g b a) with 0-255 values";
        let AstKind::Form { head, args } = &node.kind else {
            return Err(error(node, USAGE));
        };
        let alpha = match (head.as_str(), args.len()) {
            ("rgb", 3) => CExpr::Const(255.0),
            ("rgba", 4) => self.number(&args[3])?,
            _ => return Err(error(node, USAGE)),
        };
        Ok(ColorExpr([
            self.number(&args[0])?,
            self.number(&args[1])?,
            self.number(&args[2])?,
            alpha,
        ]))
    }

    fn rect(&self, node: &AstNode, args: &[AstNode]) -> ParseResult<RectItem> {
        if args.len() < 5 {
            return Err(error(node, "usage: (rect x y w h color [(rotate deg)])"));
        }
        let mut rotation = CExpr::Const(0.0);
        for option in &args[5..] {
            match &option.kind {
                AstKind::Form { head, args } if head == "rotate" && args.len() == 1 => {
                    rotation = self.number(&args[0])?;
                }
                _ => return Err(error(option, "unknown rect option (expected (rotate deg))")),
            }
        }
        Ok(RectItem {
            x: self.number(&args[0])?,
            y: self.number(&args[1])?,
            w: self.number(&args[2])?,
            h: self.number(&args[3])?,
            rotation,
            color: self.color(&args[4])?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval_rects(source: &str, time: f32) -> Vec<RectInstance> {
        let scene = Scene::compile(source).unwrap_or_else(|e| panic!("{}", e));
        let mut env = Vec::new();
        scene.fill_env(&mut env, time, 800.0, 600.0);
        let mut out = Vec::new();
        scene.build_rects(&env, &mut out);
        out
    }

    #[test]
    fn evaluates_params_defs_and_builtins() {
        let rects = eval_rects(
            "(defparam w 100 (Float 0 200 _))
             (def half {width / 2})
             (rect {half - w / 2} -10 w {time * 10} (rgb 255 0 0) (rotate 90))",
            2.0,
        );
        let r = rects[0];
        assert_eq!(r.position, [350.0, -10.0]);
        assert_eq!(r.size, [100.0, 20.0]);
        assert!((r.rotation - std::f32::consts::FRAC_PI_2).abs() < 1e-6);
        assert_eq!(r.color, [1.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn params_can_be_used_before_their_definition() {
        let rects = eval_rects("(rect x 0 1 1 (rgb 0 0 0)) (defparam x 5 (Int _ _ _))", 0.0);
        assert_eq!(rects[0].position[0], 5.0);
    }

    #[test]
    fn functions_and_left_associativity() {
        let rects = eval_rects("(rect {10 - 3 - 2} {clamp(5, 0, 3)} {max(1, 2) * -2} 0 (rgb 0 0 0))", 0.0);
        assert_eq!(rects[0].position, [5.0, 3.0]);
        assert_eq!(rects[0].size[0], -4.0);
    }

    #[test]
    fn records_param_value_span_for_write_back() {
        let source = "(defparam gap -12.5 (Float _ _ _))";
        let scene = Scene::compile(source).unwrap();
        let span = &scene.params[0].value_span;
        let text: String = source.chars().skip(span.start).take(span.end - span.start).collect();
        assert_eq!(text, "-12.5");
        assert_eq!(scene.params[0].value, -12.5);
    }

    #[test]
    fn reports_errors_with_position() {
        let source = "(rect 0 0 1 1 (rgb 0 0 0))\n(rect 0 {nope} 1 1 (rgb 0 0 0))";
        let err = Scene::compile(source).err().expect("should fail");
        assert!(err.message.contains("nope"));
        assert_eq!(parser::line_col(source, err.span.start), (2, 9));

        assert!(Scene::compile("(circle 1 2)").is_err());
        assert!(Scene::compile("(rect 0 0 1 1 (rgb 0 0))").is_err());
        assert!(Scene::compile("(def time 1)").is_err());
        assert!(Scene::compile("(rect {sin(1, 2)} 0 1 1 (rgb 0 0 0))").is_err());
    }
}
