//! UI シーン DSL のコンパイルと評価。
//!
//! リロード時に AST を `Scene` にコンパイルし、名前はスロット番号に解決しておく。
//! 毎フレームは `env`（f32 の配列）を引くだけで、名前による検索はしない。
//!
//! ```lisp
//! (defparam item_w 260 (Float 100 400 _))   ;; 調整用の値: 名前 既定値 (型 最小 最大 刻み)  `_` は指定なし
//! (var cursor 0)                            ;; 実行時の変数: 名前 初期値。リロードしても値を引き継ぐ
//! (state closed                             ;; 状態。最初に書いた state が初期状態
//!   (on (press menu) (set cursor 0) (goto open)))
//! (state open
//!   (on (press down) (set cursor {wrap(cursor + 1, 4)}))  ;; goto なし = 状態はそのまま
//!   (on (press confirm) (when {cursor == 0}) (goto closed))
//!   (on (after 2) (goto closed)))           ;; この状態に入ってから 2 秒経ったら
//! (def top {height / 2 - 120})              ;; 毎フレーム計算される名前付きの値。def 同士は使うより前に定義する
//! (background (rgb 24 24 32))
//! (rect x y w h (rgb 255 0 0) (rotate 15) (visible open))
//! ```
//! 数値には リテラル / 名前 / {中置式} が書ける。
//! 組み込みの値: `time`, `state_time`（今の状態に入ってからの秒数）, `width`, `height`（論理ピクセル）, `pi`
//! 状態の名前は、その状態にいれば 1、いなければ 0 の値として使える。
//! 関数: sin cos abs floor min max clamp lerp wrap select smoothstep
//! 操作の名前: menu up down left right confirm cancel

use crate::input::{Action, Edge, InputEvent};
use crate::parser::{self, AstKind, AstNode, BinaryOp, Expr, ParseError, ParseResult, Span, UnaryOp};
use crate::ui::TICKS_PER_SECOND;
use crate::ui::renderer::RectInstance;

const SLOT_TIME: usize = 0;
const SLOT_WIDTH: usize = 1;
const SLOT_HEIGHT: usize = 2;
const SLOT_STATE_TIME: usize = 3;
const BUILTIN_NAMES: [&str; 4] = ["time", "width", "height", "state_time"];

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

/// コンパイル済みのシーン（定義）。実行時の状態は持たない（→ `Runtime`）
///
/// スロットの並び: `[組み込み値, パラメータ..., 変数..., 状態フラグ..., def...]`
pub struct Scene {
    pub params: Vec<Param>,
    vars: Vec<VarDef>,
    states: Vec<StateDef>,
    defs: Vec<CExpr>,
    background: Option<ColorExpr>,
    rects: Vec<RectItem>,
}

struct VarDef {
    name: String,
    initial: f32,
}

struct StateDef {
    name: String,
    transitions: Vec<Transition>,
}

struct Transition {
    trigger: Trigger,
    guard: Option<CExpr>,
    /// (変数番号, 値)
    sets: Vec<(usize, CExpr)>,
    target: Option<usize>,
    /// ログ用の説明（例: "press down"）
    label: String,
}

enum Trigger {
    Input(InputEvent),
    /// 状態に入ってからの秒数
    After(CExpr),
}

/// 実行時の状態。定義（`Scene`）とは別に持ち、リロードしても名前で引き継ぐ
#[derive(Debug, Clone, Default)]
pub struct Runtime {
    state: Option<usize>,
    entered_tick: u64,
    vars: Vec<f32>,
}

/// `Scene::step` で起きたこと。ログ（将来は操作ログ）に使う
#[derive(Debug, PartialEq)]
pub enum StepEvent<'a> {
    Transition { from: &'a str, to: &'a str, label: &'a str },
    /// goto のない遷移（変数だけ変わった）
    Internal { state: &'a str, label: &'a str },
    /// どの遷移にも当てはまらなかった押下（「入力が無視された」）
    Ignored { state: &'a str, input: InputEvent },
}

struct RectItem {
    x: CExpr,
    y: CExpr,
    w: CExpr,
    h: CExpr,
    /// 度
    rotation: CExpr,
    color: ColorExpr,
    visible: Option<CExpr>,
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
    Wrap,
    Select,
    Smoothstep,
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
            // wrap(x, n): 0..n に収める（負の数も正しく折り返す）
            "wrap" => (Func::Wrap, 2),
            // select(条件, 真のとき, 偽のとき)
            "select" => (Func::Select, 3),
            // smoothstep(端0, 端1, x)
            "smoothstep" => (Func::Smoothstep, 3),
            _ => return None,
        })
    }
}

fn bool_value(b: bool) -> f32 {
    if b { 1.0 } else { 0.0 }
}

impl CExpr {
    fn eval(&self, env: &[f32]) -> f32 {
        match self {
            CExpr::Const(v) => *v,
            CExpr::Slot(i) => env[*i],
            CExpr::Neg(e) => -e.eval(env),
            CExpr::Not(e) => bool_value(e.eval(env) == 0.0),
            CExpr::Binary(op, l, r) => {
                let (l, r) = (l.eval(env), r.eval(env));
                match op {
                    BinaryOp::Eq => bool_value(l == r),
                    BinaryOp::Ne => bool_value(l != r),
                    BinaryOp::Lt => bool_value(l < r),
                    BinaryOp::Le => bool_value(l <= r),
                    BinaryOp::Gt => bool_value(l > r),
                    BinaryOp::Ge => bool_value(l >= r),
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
                    Func::Wrap => a(0).rem_euclid(a(1)),
                    Func::Select => {
                        if a(0) != 0.0 { a(1) } else { a(2) }
                    }
                    Func::Smoothstep => {
                        let t = ((a(2) - a(0)) / (a(1) - a(0))).clamp(0.0, 1.0);
                        t * t * (3.0 - 2.0 * t)
                    }
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

fn seconds(ticks: u64) -> f32 {
    ticks as f32 / TICKS_PER_SECOND as f32
}

impl Scene {
    pub fn compile(source: &str) -> ParseResult<Scene> {
        let nodes = parser::parse(source)?;
        let forms = nodes
            .iter()
            .map(|node| match &node.kind {
                AstKind::Form { head, args } => Ok((node, head.as_str(), args.as_slice())),
                _ => Err(error(node, "expected a form like `(rect ...)`")),
            })
            .collect::<ParseResult<Vec<_>>>()?;

        // 1周目: パラメータ・変数・状態の名前を集める（どこからでも参照できるように）
        let mut params = Vec::new();
        let mut vars = Vec::new();
        let mut states = Vec::new(); // (名前, 名前のノード)
        for &(node, head, args) in &forms {
            match head {
                "defparam" => params.push((compile_defparam(args, node)?, node)),
                "var" => {
                    let [name, initial] = args else {
                        return Err(error(node, "usage: (var name initial)"));
                    };
                    let Some(initial) = literal_number(initial) else {
                        return Err(error(initial, "initial value must be a number literal"));
                    };
                    vars.push((VarDef { name: symbol(name)?.to_string(), initial }, name));
                }
                "state" => {
                    let Some(name) = args.first() else {
                        return Err(error(node, "usage: (state name (on ...)...)"));
                    };
                    states.push((symbol(name)?.to_string(), name));
                }
                _ => {}
            }
        }

        // スロットの並びどおりに名前を登録する
        let mut compiler = Compiler {
            names: BUILTIN_NAMES.iter().map(|s| s.to_string()).collect(),
            var_base: BUILTIN_NAMES.len() + params.len(),
            var_count: vars.len(),
            state_names: states.iter().map(|(name, _)| name.clone()).collect(),
        };
        for (param, node) in &params {
            compiler.declare(&param.name, node)?;
        }
        for (var, node) in &vars {
            compiler.declare(&var.name, node)?;
        }
        for (name, node) in &states {
            compiler.declare(name, node)?;
        }
        let params: Vec<Param> = params.into_iter().map(|(p, _)| p).collect();
        let vars: Vec<VarDef> = vars.into_iter().map(|(v, _)| v).collect();

        // 2周目: def（上から順に。def 同士は使うより前に定義する）
        let mut defs = Vec::new();
        for &(node, head, args) in &forms {
            if head == "def" {
                let [name, value] = args else {
                    return Err(error(node, "usage: (def name value)"));
                };
                defs.push(compiler.number(value)?);
                compiler.declare(symbol(name)?, name)?;
            }
        }

        // 3周目: 状態の中身と描画要素
        let mut states = Vec::new();
        let mut background = None;
        let mut rects = Vec::new();
        for &(node, head, args) in &forms {
            match head {
                "defparam" | "var" | "def" => {}
                "state" => states.push(compiler.state(args)?),
                "background" => {
                    let [color] = args else {
                        return Err(error(node, "usage: (background color)"));
                    };
                    background = Some(compiler.color(color)?);
                }
                "rect" => rects.push(compiler.rect(node, args)?),
                other => return Err(error(node, format!("unknown form `{}`", other))),
            }
        }

        Ok(Scene { params, vars, states, defs, background, rects })
    }

    /// 実行時の状態を作る。前のシーンの状態があれば、同じ名前の状態・変数を引き継ぐ
    pub fn adopt(&self, previous: Option<(&Scene, &Runtime)>, tick: u64) -> Runtime {
        let mut runtime = Runtime {
            state: if self.states.is_empty() { None } else { Some(0) },
            entered_tick: tick,
            vars: self.vars.iter().map(|v| v.initial).collect(),
        };
        let Some((old, old_runtime)) = previous else {
            return runtime;
        };
        for (value, var) in runtime.vars.iter_mut().zip(&self.vars) {
            if let Some(i) = old.vars.iter().position(|v| v.name == var.name) {
                *value = old_runtime.vars[i];
            }
        }
        let old_state = old_runtime.state.map(|s| &old.states[s].name);
        if let Some(i) = self.states.iter().position(|s| Some(&s.name) == old_state) {
            runtime.state = Some(i);
            runtime.entered_tick = old_runtime.entered_tick;
        }
        runtime
    }

    /// 1 tick 分進める: 入力を順に処理し、最後に時間経過の遷移を調べる
    pub fn step(
        &self,
        runtime: &mut Runtime,
        tick: u64,
        inputs: &[InputEvent],
        logical_size: [f32; 2],
        env: &mut Vec<f32>,
        report: &mut dyn FnMut(StepEvent),
    ) {
        for input in inputs {
            self.fire(runtime, tick, Some(*input), logical_size, env, report);
        }
        self.fire(runtime, tick, None, logical_size, env, report);
    }

    /// 今の状態で最初に当てはまった遷移を1つだけ実行する
    fn fire(
        &self,
        runtime: &mut Runtime,
        tick: u64,
        input: Option<InputEvent>,
        logical_size: [f32; 2],
        env: &mut Vec<f32>,
        report: &mut dyn FnMut(StepEvent),
    ) {
        let Some(current) = runtime.state else {
            return;
        };
        self.fill_env(env, runtime, tick, logical_size);
        let state = &self.states[current];

        let fired = state.transitions.iter().find(|t| {
            let triggered = match (&t.trigger, input) {
                (Trigger::Input(expected), Some(input)) => *expected == input,
                (Trigger::After(secs), None) => env[SLOT_STATE_TIME] >= secs.eval(env),
                _ => false,
            };
            triggered && t.guard.as_ref().is_none_or(|g| g.eval(env) != 0.0)
        });
        let Some(transition) = fired else {
            if let Some(input) = input.filter(|i| i.edge == Edge::Press) {
                report(StepEvent::Ignored { state: &state.name, input });
            }
            return;
        };

        // 全部の値を同じ環境で評価してから代入する（set を書く順番に結果が左右されないように）
        let values: Vec<f32> = transition.sets.iter().map(|(_, value)| value.eval(env)).collect();
        for ((var, _), value) in transition.sets.iter().zip(values) {
            runtime.vars[*var] = value;
        }
        match transition.target {
            Some(target) => {
                runtime.state = Some(target);
                runtime.entered_tick = tick;
                report(StepEvent::Transition {
                    from: &state.name,
                    to: &self.states[target].name,
                    label: &transition.label,
                });
            }
            None => report(StepEvent::Internal { state: &state.name, label: &transition.label }),
        }
    }

    /// 評価に使う環境を作る（スロットの並びは `Scene` のコメント参照）
    pub fn fill_env(&self, env: &mut Vec<f32>, runtime: &Runtime, tick: u64, logical_size: [f32; 2]) {
        env.clear();
        env.resize(BUILTIN_NAMES.len(), 0.0);
        env[SLOT_TIME] = seconds(tick);
        env[SLOT_WIDTH] = logical_size[0];
        env[SLOT_HEIGHT] = logical_size[1];
        env[SLOT_STATE_TIME] = seconds(tick.saturating_sub(runtime.entered_tick));
        env.extend(self.params.iter().map(|p| p.value));
        env.extend_from_slice(&runtime.vars);
        env.extend((0..self.states.len()).map(|i| bool_value(runtime.state == Some(i))));
        for def in &self.defs {
            let value = def.eval(env);
            env.push(value);
        }
    }

    pub fn background(&self, env: &[f32]) -> Option<[f32; 4]> {
        self.background.as_ref().map(|c| c.eval(env))
    }

    pub fn build_rects(&self, env: &[f32], out: &mut Vec<RectInstance>) {
        let visible = |r: &&RectItem| r.visible.as_ref().is_none_or(|v| v.eval(env) != 0.0);
        out.extend(self.rects.iter().filter(visible).map(|r| RectInstance {
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

/// フォームなら (head, args) を返す。`match` で `Some(("set", [var, value]))` のように使う
fn form_parts(node: &AstNode) -> Option<(&str, &[AstNode])> {
    match &node.kind {
        AstKind::Form { head, args } => Some((head.as_str(), args.as_slice())),
        _ => None,
    }
}

fn symbol(node: &AstNode) -> ParseResult<&str> {
    match &node.kind {
        AstKind::Symbol(name) => Ok(name),
        _ => Err(error(node, "expected a name")),
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
    let name = symbol(name)?;
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
        name: name.to_string(),
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
    /// 変数のスロットは var_base から var_count 個
    var_base: usize,
    var_count: usize,
    state_names: Vec<String>,
}

impl Compiler {
    fn declare(&mut self, name: &str, node: &AstNode) -> ParseResult<()> {
        if name == "pi" || self.names.iter().any(|n| n == name) {
            return Err(error(node, format!("`{}` is already defined", name)));
        }
        self.names.push(name.to_string());
        Ok(())
    }

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
            return Err(error(node, "usage: (rect x y w h color [(rotate deg)] [(visible cond)])"));
        }
        let mut rotation = CExpr::Const(0.0);
        let mut visible = None;
        for option in &args[5..] {
            match form_parts(option) {
                Some(("rotate", [value])) => rotation = self.number(value)?,
                Some(("visible", [value])) => visible = Some(self.number(value)?),
                _ => return Err(error(option, "unknown rect option (expected (rotate deg) or (visible cond))")),
            }
        }
        Ok(RectItem {
            x: self.number(&args[0])?,
            y: self.number(&args[1])?,
            w: self.number(&args[2])?,
            h: self.number(&args[3])?,
            rotation,
            color: self.color(&args[4])?,
            visible,
        })
    }

    fn state_index(&self, node: &AstNode) -> ParseResult<usize> {
        let name = symbol(node)?;
        self.state_names
            .iter()
            .position(|n| n == name)
            .ok_or_else(|| error(node, format!("unknown state `{}`", name)))
    }

    fn state(&self, args: &[AstNode]) -> ParseResult<StateDef> {
        let name = symbol(&args[0])?.to_string();
        let transitions = args[1..].iter().map(|node| self.transition(node)).collect::<ParseResult<_>>()?;
        Ok(StateDef { name, transitions })
    }

    /// (on TRIGGER [(when cond)] [(set var value)]... [(goto state)])
    fn transition(&self, node: &AstNode) -> ParseResult<Transition> {
        const USAGE: &str = "usage: (on (press|release action | after seconds) [(when cond)] [(set var value)]... [(goto state)])";
        let AstKind::Form { head, args } = &node.kind else {
            return Err(error(node, USAGE));
        };
        if head != "on" || args.is_empty() {
            return Err(error(node, USAGE));
        }

        let trigger_node = &args[0];
        let AstKind::Form { head: kind, args: trigger_args } = &trigger_node.kind else {
            return Err(error(trigger_node, USAGE));
        };
        let [trigger_arg] = trigger_args.as_slice() else {
            return Err(error(trigger_node, USAGE));
        };
        let (trigger, label) = match kind.as_str() {
            "press" | "release" => {
                let action_name = symbol(trigger_arg)?;
                let action = Action::from_name(action_name)
                    .ok_or_else(|| error(trigger_arg, format!("unknown action `{}`", action_name)))?;
                let edge = if kind == "press" { Edge::Press } else { Edge::Release };
                (Trigger::Input(InputEvent { action, edge }), format!("{} {}", kind, action_name))
            }
            "after" => (Trigger::After(self.number(trigger_arg)?), "after".to_string()),
            _ => return Err(error(trigger_node, USAGE)),
        };

        let mut guard = None;
        let mut sets = Vec::new();
        let mut target = None;
        for clause in &args[1..] {
            match form_parts(clause) {
                Some(("when", [cond])) if guard.is_none() => guard = Some(self.number(cond)?),
                Some(("set", [var, value])) => {
                    let var_name = symbol(var)?;
                    let index = match self.names.iter().position(|n| n == var_name) {
                        Some(slot) if (self.var_base..self.var_base + self.var_count).contains(&slot) => {
                            slot - self.var_base
                        }
                        _ => return Err(error(var, format!("`{}` is not a var", var_name))),
                    };
                    sets.push((index, self.number(value)?));
                }
                Some(("goto", [state])) if target.is_none() => target = Some(self.state_index(state)?),
                _ => return Err(error(clause, USAGE)),
            }
        }
        // goto のない after は条件を満たしている間、毎 tick 発火してしまうので禁止
        if matches!(trigger, Trigger::After(_)) && target.is_none() {
            return Err(error(node, "(after ...) needs a (goto state)"));
        }

        Ok(Transition { trigger, guard, sets, target, label })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::Action;

    const SIZE: [f32; 2] = [800.0, 600.0];

    fn compile(source: &str) -> Scene {
        Scene::compile(source).unwrap_or_else(|e| panic!("{} at {:?}", e, parser::line_col(source, e.span.start)))
    }

    fn eval_rects(source: &str, tick: u64) -> Vec<RectInstance> {
        let scene = compile(source);
        let runtime = scene.adopt(None, 0);
        let mut env = Vec::new();
        scene.fill_env(&mut env, &runtime, tick, SIZE);
        let mut out = Vec::new();
        scene.build_rects(&env, &mut out);
        out
    }

    fn press(action: Action) -> InputEvent {
        InputEvent { action, edge: Edge::Press }
    }

    fn release(action: Action) -> InputEvent {
        InputEvent { action, edge: Edge::Release }
    }

    /// 1 tick 進めて、起きたことを文字列で返す
    fn step(scene: &Scene, runtime: &mut Runtime, tick: u64, inputs: &[InputEvent]) -> Vec<String> {
        let mut env = Vec::new();
        let mut log = Vec::new();
        scene.step(runtime, tick, inputs, SIZE, &mut env, &mut |e| log.push(format!("{:?}", e)));
        log
    }

    fn state_name<'a>(scene: &'a Scene, runtime: &Runtime) -> &'a str {
        &scene.states[runtime.state.unwrap()].name
    }

    #[test]
    fn evaluates_params_defs_and_builtins() {
        let rects = eval_rects(
            "(defparam w 100 (Float 0 200 _))
             (def half {width / 2})
             (rect {half - w / 2} -10 w {time * 10} (rgb 255 0 0) (rotate 90))",
            120,
        );
        let r = rects[0];
        assert_eq!(r.position, [350.0, -10.0]);
        assert_eq!(r.size, [100.0, 20.0]);
        assert!((r.rotation - std::f32::consts::FRAC_PI_2).abs() < 1e-6);
        assert_eq!(r.color, [1.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn names_can_be_used_before_their_definition() {
        let rects = eval_rects(
            "(rect x d 1 1 (rgb 0 0 0)) (def d {x * 2}) (defparam x 5 (Int _ _ _))",
            0,
        );
        assert_eq!(rects[0].position, [5.0, 10.0]);
    }

    #[test]
    fn operators_and_functions() {
        let rects = eval_rects(
            "(rect {10 - 3 - 2} {clamp(5, 0, 3)} {max(1, 2) * -2} {1 + 1 == 2} (rgb 0 0 0))
             (rect {wrap(-1, 4)} {select(2 > 3, 10, 20)} {3 <= 3} {!(1 != 1)} (rgb 0 0 0))",
            0,
        );
        assert_eq!(rects[0].position, [5.0, 3.0]);
        assert_eq!(rects[0].size, [-4.0, 1.0]);
        assert_eq!(rects[1].position, [3.0, 20.0]);
        assert_eq!(rects[1].size, [1.0, 1.0]);
    }

    #[test]
    fn visible_option_filters_rects() {
        let rects = eval_rects("(rect 0 0 1 1 (rgb 0 0 0) (visible 0)) (rect 1 0 1 1 (rgb 0 0 0))", 0);
        assert_eq!(rects.len(), 1);
        assert_eq!(rects[0].position[0], 1.0);
    }

    #[test]
    fn records_param_value_span_for_write_back() {
        let source = "(defparam gap -12.5 (Float _ _ _))";
        let scene = compile(source);
        let span = &scene.params[0].value_span;
        let text: String = source.chars().skip(span.start).take(span.end - span.start).collect();
        assert_eq!(text, "-12.5");
        assert_eq!(scene.params[0].value, -12.5);
    }

    const MENU: &str = "
        (var cursor 0)
        (state closed
          (on (press menu) (set cursor 0) (goto open)))
        (state open
          (on (press down) (set cursor {wrap(cursor + 1, 3)}))
          (on (press confirm) (when {cursor == 2}) (goto closed))
          (on (release menu) (goto closed))
          (on (after 1) (goto closed)))
        (rect cursor open 1 1 (rgb 0 0 0))";

    #[test]
    fn state_machine_transitions_and_vars() {
        let scene = compile(MENU);
        let mut rt = scene.adopt(None, 0);
        assert_eq!(state_name(&scene, &rt), "closed");

        let log = step(&scene, &mut rt, 1, &[press(Action::Down)]);
        assert_eq!(log, vec!["Ignored { state: \"closed\", input: InputEvent { action: Down, edge: Press } }"]);

        step(&scene, &mut rt, 2, &[press(Action::Menu)]);
        assert_eq!(state_name(&scene, &rt), "open");

        // ガードが偽のあいだは決定しても閉じない
        let log = step(&scene, &mut rt, 3, &[press(Action::Confirm)]);
        assert!(log[0].starts_with("Ignored"));
        step(&scene, &mut rt, 4, &[press(Action::Down), press(Action::Down), press(Action::Down)]);
        assert_eq!(rt.vars[0], 0.0); // 0 → 1 → 2 → 0（折り返し）
        step(&scene, &mut rt, 5, &[press(Action::Down), press(Action::Down), press(Action::Confirm)]);
        assert_eq!(state_name(&scene, &rt), "closed");

        // 式から状態と変数を参照できる
        let mut env = Vec::new();
        scene.fill_env(&mut env, &rt, 5, SIZE);
        let mut rects = Vec::new();
        scene.build_rects(&env, &mut rects);
        assert_eq!(rects[0].position, [2.0, 0.0]);
    }

    #[test]
    fn after_fires_on_state_time() {
        let scene = compile(MENU);
        let mut rt = scene.adopt(None, 0);
        step(&scene, &mut rt, 10, &[press(Action::Menu)]);
        step(&scene, &mut rt, 10 + TICKS_PER_SECOND as u64 - 1, &[]);
        assert_eq!(state_name(&scene, &rt), "open");
        let log = step(&scene, &mut rt, 10 + TICKS_PER_SECOND as u64, &[]);
        assert_eq!(state_name(&scene, &rt), "closed");
        assert!(log[0].contains("label: \"after\""));
    }

    #[test]
    fn reload_keeps_state_and_vars_by_name() {
        let scene = compile(MENU);
        let mut rt = scene.adopt(None, 0);
        step(&scene, &mut rt, 1, &[press(Action::Menu), press(Action::Down)]);

        // 変数を足し、状態の順番を入れ替えたシーンに差し替える
        let reloaded = compile(
            "(var extra 7) (var cursor 0)
             (state open (on (release menu) (goto closed)))
             (state closed (on (press menu) (goto open)))",
        );
        let rt2 = reloaded.adopt(Some((&scene, &rt)), 50);
        assert_eq!(state_name(&reloaded, &rt2), "open");
        assert_eq!(rt2.vars, vec![7.0, 1.0]);
        assert_eq!(rt2.entered_tick, 1);

        // 今の状態が消えたら初期状態から
        let gone = compile("(state idle)");
        let rt3 = gone.adopt(Some((&scene, &rt)), 50);
        assert_eq!(state_name(&gone, &rt3), "idle");
        assert_eq!(rt3.entered_tick, 50);
    }

    /// 同梱のシーンファイルが壊れていないこと、主な操作の流れが想定どおりであること
    #[test]
    fn bundled_menu_a_hold_and_direction() {
        let scene = compile(include_str!("../../ui/menu_a.ui"));
        let mut rt = scene.adopt(None, 0);
        // 押してすぐ離すと何も開かない
        step(&scene, &mut rt, 1, &[press(Action::Menu)]);
        step(&scene, &mut rt, 2, &[release(Action::Menu)]);
        assert_eq!(state_name(&scene, &rt), "closed");
        // 押したまま右 → 画面1
        step(&scene, &mut rt, 3, &[press(Action::Menu)]);
        step(&scene, &mut rt, 4, &[press(Action::Right)]);
        step(&scene, &mut rt, 5, &[release(Action::Menu)]);
        assert_eq!((state_name(&scene, &rt), rt.vars[0]), ("page", 1.0));
        step(&scene, &mut rt, 6, &[press(Action::Cancel)]);
        assert_eq!(state_name(&scene, &rt), "closed");
    }

    #[test]
    fn bundled_menu_b_select_and_confirm() {
        let scene = compile(include_str!("../../ui/menu_b.ui"));
        let mut rt = scene.adopt(None, 0);
        step(&scene, &mut rt, 1, &[press(Action::Menu), release(Action::Menu)]);
        step(&scene, &mut rt, 2, &[press(Action::Up)]); // 0 → 3（折り返し）
        step(&scene, &mut rt, 3, &[press(Action::Up), press(Action::Confirm)]);
        assert_eq!(state_name(&scene, &rt), "page");
        assert_eq!(rt.vars, vec![2.0, 2.0]); // cursor, screen
        step(&scene, &mut rt, 4, &[press(Action::Cancel)]);
        assert_eq!(state_name(&scene, &rt), "open");
    }

    #[test]
    fn reports_errors_with_position() {
        let source = "(rect 0 0 1 1 (rgb 0 0 0))\n(rect 0 {nope} 1 1 (rgb 0 0 0))";
        let err = Scene::compile(source).err().expect("should fail");
        assert!(err.message.contains("nope"));
        assert_eq!(parser::line_col(source, err.span.start), (2, 9));

        for bad in [
            "(circle 1 2)",
            "(rect 0 0 1 1 (rgb 0 0))",
            "(def time 1)",
            "(rect {sin(1, 2)} 0 1 1 (rgb 0 0 0))",
            "(state a (on (press jump) (goto a)))",
            "(state a (on (press menu) (goto b)))",
            "(state a (on (after 1)))",
            "(defparam p 1 (Float)) (state a (on (press menu) (set p 2)))",
            "(var x 0) (state x)",
        ] {
            assert!(Scene::compile(bad).is_err(), "should fail: {}", bad);
        }
    }
}
