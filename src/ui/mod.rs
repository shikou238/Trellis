//! UI シーン（DSL）の読み込み・ホットリロード・評価。

pub mod renderer;
pub mod scene;

use std::time::{Duration, Instant, SystemTime};

use crate::{assets, parser};
use renderer::RectInstance;
use scene::Scene;

/// 固定タイムステップ（決定性のため、時間は tick 数から求める）
pub const TICKS_PER_SECOND: u32 = 60;
const TICK: Duration = Duration::from_nanos(1_000_000_000 / TICKS_PER_SECOND as u64);
/// 処理落ちのときに追いつこうとして固まらないための上限
const MAX_CATCH_UP: Duration = Duration::from_millis(250);
const FILE_POLL_INTERVAL: Duration = Duration::from_millis(200);

/// 1 つのシーンファイルを動かす。
/// 定義（`scene`）はリロードで差し替え、実行時状態（`tick`）は保持する
pub struct ScenePlayer {
    path: String,
    scene: Option<Scene>,
    last_modified: Option<SystemTime>,
    last_poll: Instant,
    clock: Instant,
    accumulator: Duration,
    tick: u64,
    env: Vec<f32>,
}

impl ScenePlayer {
    pub fn new(path: &str) -> Self {
        let mut player = Self {
            path: path.to_string(),
            scene: None,
            last_modified: None,
            last_poll: Instant::now(),
            clock: Instant::now(),
            accumulator: Duration::ZERO,
            tick: 0,
            env: Vec::new(),
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
                log::error!("{:#}", e);
                return;
            }
        };
        match Scene::compile(&source) {
            Ok(scene) => {
                log::info!("loaded {}", self.path);
                self.scene = Some(scene);
            }
            Err(e) => {
                let (line, col) = parser::line_col(&source, e.span.start);
                log::error!("{}:{}:{}: {}", self.path, line, col, e.message);
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

    /// 経過した実時間ぶん tick を進める
    pub fn advance(&mut self) {
        let now = Instant::now();
        self.accumulator = (self.accumulator + (now - self.clock)).min(MAX_CATCH_UP);
        self.clock = now;
        while self.accumulator >= TICK {
            self.accumulator -= TICK;
            self.tick += 1;
        }
    }

    pub fn time(&self) -> f32 {
        self.tick as f32 / TICKS_PER_SECOND as f32
    }

    /// 現在のシーンを評価して矩形を `out` に追加し、背景色（線形 RGBA）を返す
    pub fn build(&mut self, logical_size: [f32; 2], out: &mut Vec<RectInstance>) -> Option<[f32; 4]> {
        let time = self.time();
        let scene = self.scene.as_ref()?;
        scene.fill_env(&mut self.env, time, logical_size[0], logical_size[1]);
        scene.build_rects(&self.env, out);
        scene.background(&self.env)
    }
}
