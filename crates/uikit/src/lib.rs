//! UI シーン（DSL）の読み込み・ホットリロード・実行と、パラメータのインスペクタ。個々の製品は知らない

pub mod inspector;
pub mod scene;

use std::time::{Duration, Instant, SystemTime};

use dsl::parser;
use engine::assets;
use engine::input::InputEvent;
use engine::renderer::RectInstance;
use scene::{Runtime, Scene, StepEvent};

/// 固定タイムステップ（決定性のため、時間は tick 数から求める）
pub const TICKS_PER_SECOND: u32 = 60;
const TICK: Duration = Duration::from_nanos(1_000_000_000 / TICKS_PER_SECOND as u64);
/// 処理落ちのときに追いつこうとして固まらないための上限
const MAX_CATCH_UP: Duration = Duration::from_millis(250);
const FILE_POLL_INTERVAL: Duration = Duration::from_millis(200);

/// 1 つのシーンファイルを動かす。
/// 定義（`scene`）はリロードで差し替え、実行時の状態（`tick`, `runtime`）は保持する
pub struct ScenePlayer {
    path: String,
    scene: Option<Scene>,
    runtime: Runtime,
    last_modified: Option<SystemTime>,
    last_poll: Instant,
    clock: FixedClock,
    tick: u64,
    /// 次の tick で処理する入力
    pending_inputs: Vec<InputEvent>,
    env: Vec<f32>,
    last_error: Option<String>,
    /// 最後に読み込みに成功したソース
    source: String,
}

impl ScenePlayer {
    pub fn new(path: &str) -> Self {
        let mut player = Self {
            path: path.to_string(),
            scene: None,
            runtime: Runtime::default(),
            last_modified: None,
            last_poll: Instant::now(),
            clock: FixedClock::new(),
            tick: 0,
            pending_inputs: Vec::new(),
            env: Vec::new(),
            last_error: None,
            source: String::new(),
        };
        player.reload();
        player
    }

    /// ファイルを読み直す。失敗したら直前の正常なシーンのまま動き続ける
    pub fn reload(&mut self) {
        self.last_modified = assets::modified(&self.path);
        let source = match assets::read_to_string(&self.path) {
            Ok(source) => source,
            Err(e) => {
                let message = format!("{:#}", e);
                log::error!("{}", message);
                self.last_error = Some(message);
                return;
            }
        };
        match Scene::compile(&source) {
            Ok(scene) => {
                log::info!("loaded {}", self.path);
                let previous = self.scene.as_ref().map(|old| (old, &self.runtime));
                self.runtime = scene.adopt(previous, self.tick);
                self.scene = Some(scene);
                self.last_error = None;
                self.source = source;
            }
            Err(e) => {
                let (line, col) = parser::line_col(&source, e.span.start);
                let message = format!("{}:{}:{}: {}", self.path, line, col, e.message);
                log::error!("{}", message);
                self.last_error = Some(message);
            }
        }
    }

    /// ファイルが保存されていたらリロードする
    pub fn poll_file(&mut self) {
        if self.last_poll.elapsed() < FILE_POLL_INTERVAL {
            return;
        }
        self.last_poll = Instant::now();
        if assets::modified(&self.path) != self.last_modified {
            self.reload();
        }
    }

    pub fn queue_inputs(&mut self, inputs: &[InputEvent]) {
        self.pending_inputs.extend_from_slice(inputs);
    }

    /// 経過した実時間ぶん tick を進める。たまっている入力は最初の tick で処理する
    pub fn advance(&mut self, logical_size: [f32; 2]) {
        for _ in 0..self.clock.ticks_due() {
            let inputs = std::mem::take(&mut self.pending_inputs);
            let (path, tick) = (self.path.clone(), self.tick + 1);
            self.step_tick(&inputs, logical_size, &mut |event| log_step_event(&path, tick, event));
        }
    }

    /// 1 tick 進める。計測（`session.rs`）は自分の時計でこれを呼び、起きたことを `report` で受け取る
    pub fn step_tick(&mut self, inputs: &[InputEvent], logical_size: [f32; 2], report: &mut dyn FnMut(StepEvent)) {
        self.tick += 1;
        if let Some(scene) = &self.scene {
            scene.step(&mut self.runtime, self.tick, inputs, logical_size, &mut self.env, report);
        }
    }

    /// 実行時の状態を初期状態に戻す（計測の試行ごとに条件をそろえるため）。定義とパラメータはそのまま
    pub fn reset(&mut self) {
        self.pending_inputs.clear();
        if let Some(scene) = &self.scene {
            self.runtime = scene.adopt(None, self.tick);
        }
    }

    /// パラメータの (名前, 値) の一覧。計測中の変更を検出してログに残すために使う
    pub fn param_values(&self) -> Vec<(String, f32)> {
        self.params().iter().map(|p| (p.name.clone(), p.value)).collect()
    }

    /// 今読み込んでいるソース。計測のログにハッシュを残すために使う
    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn params(&self) -> &[scene::Param] {
        self.scene.as_ref().map_or(&[], |s| &s.params)
    }

    pub fn state_name(&self) -> Option<&str> {
        self.scene.as_ref()?.state_name(&self.runtime)
    }

    /// 直近のリロードの失敗理由。成功したら消える
    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    /// メモリ上の値だけ変える（スライダーのドラッグ中）
    pub fn set_param(&mut self, name: &str, value: f32) {
        if let Some(param) = self.scene.as_mut().and_then(|s| s.param_mut(name)) {
            param.value = param.snap(value);
        }
    }

    /// ファイルに書き戻す（スライダーを離したとき）。保存を検知して自動でリロードされる
    pub fn write_param(&mut self, name: &str, value: f32) {
        let result = assets::read_to_string(&self.path).and_then(|source| {
            let rewritten = Scene::rewrite_param(&source, name, value).map_err(|e| {
                let (line, col) = parser::line_col(&source, e.span.start);
                anyhow::anyhow!("{}:{}:{}: {}", self.path, line, col, e.message)
            })?;
            assets::write_string(&self.path, &rewritten)
        });
        match result {
            Ok(()) => log::info!("{}: wrote {} = {}", self.path, name, value),
            Err(e) => log::error!("could not write back {}: {:#}", name, e),
        }
    }

    /// 現在のシーンを評価して矩形を `out` に追加し、背景色（線形 RGBA）を返す
    pub fn build(&mut self, logical_size: [f32; 2], out: &mut Vec<RectInstance>) -> Option<[f32; 4]> {
        let scene = self.scene.as_ref()?;
        scene.fill_env(&mut self.env, &self.runtime, self.tick, logical_size);
        scene.build_rects(&self.env, out);
        scene.background(&self.env)
    }
}

/// 普段の操作ではログに出すだけ。計測中の記録は `session.rs` が行う
fn log_step_event(path: &str, tick: u64, event: StepEvent) {
    match event {
        StepEvent::Transition { from, to, label } => {
            log::info!("{} [tick {}] {} -> {} ({})", path, tick, from, to, label)
        }
        StepEvent::Internal { state, label } => log::info!("{} [tick {}] {} ({})", path, tick, state, label),
        StepEvent::Ignored { state, input } => {
            log::info!("{} [tick {}] {}: ignored press {}", path, tick, state, input.action.name())
        }
        StepEvent::Emit { name, args } => log::info!("{} [tick {}] emit {} {:?}", path, tick, name, args),
    }
}

/// 実時間を固定タイムステップの tick 数に変換する
pub struct FixedClock {
    last: Instant,
    accumulator: Duration,
}

impl FixedClock {
    pub fn new() -> Self {
        Self { last: Instant::now(), accumulator: Duration::ZERO }
    }

    /// 前回呼んでから進めるべき tick 数
    pub fn ticks_due(&mut self) -> u32 {
        let now = Instant::now();
        self.accumulator = (self.accumulator + (now - self.last)).min(MAX_CATCH_UP);
        self.last = now;
        let mut ticks = 0;
        while self.accumulator >= TICK {
            self.accumulator -= TICK;
            ticks += 1;
        }
        ticks
    }
}
