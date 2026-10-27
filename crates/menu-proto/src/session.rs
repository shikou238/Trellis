//! 計測モード。
//! 1 つのウィンドウで、設定ファイル（`ui/tasks.ui`）に並べたシーンを試行ごとに交互に動かし、
//! 課題（「青」など）を出して、完了・誤操作・時間切れまでを `logs/` にログとして記録する。
//!
//! 1 試行の流れ: シーンを初期状態に戻す → wait 秒待つ → 課題を表示 →
//!   シーンが期待どおりの `(emit ...)` を出したら完了 / 同じ名前で違う値なら誤操作 / timeout 秒で時間切れ
//!
//! ```lisp
//! (scenes "ui/menu_a.ui" "ui/menu_b.ui")   ;; 交互に試すシーン
//! (trials 20)                             ;; 試行の回数（全シーンの合計）
//! (timeout 10)                            ;; 1 試行の制限時間（秒）
//! (wait 0.5)                              ;; 試行の間の待ち時間（秒）
//! (task "青" (opened 1))                  ;; 画面に出す文字と、完了とみなす通知
//! ```

use std::io::Write;

use engine::input::{Edge, InputEvent};
use dsl::parser::{self, AstKind, AstNode, ParseError, ParseResult};
use crate::analysis::{TrialResult, quote};
use engine::renderer::{RectInstance, srgb8};
use uikit::scene::StepEvent;
use engine::text::Label;
use engine::assets;
use uikit::{FixedClock, ScenePlayer, TICKS_PER_SECOND};

pub const CONFIG_PATH: &str = "ui/tasks.ui";

#[derive(Debug, Clone, PartialEq)]
pub struct Task {
    /// 画面に出す文字
    pub name: String,
    /// 完了とみなす通知の名前と値
    pub expect_name: String,
    pub expect_args: Vec<f32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TestConfig {
    pub scenes: Vec<String>,
    pub tasks: Vec<Task>,
    pub trials: u32,
    pub timeout: f32,
    pub wait: f32,
}

fn error(node: &AstNode, message: impl Into<String>) -> ParseError {
    ParseError { message: message.into(), span: node.span.clone() }
}

fn number(node: &AstNode) -> ParseResult<f32> {
    match node.kind {
        AstKind::Integer(v) => Ok(v as f32),
        AstKind::Float(v) => Ok(v as f32),
        _ => Err(error(node, "expected a number")),
    }
}

fn string(node: &AstNode) -> ParseResult<String> {
    match &node.kind {
        AstKind::String(s) => Ok(s.clone()),
        _ => Err(error(node, "expected a string")),
    }
}

impl TestConfig {
    pub fn compile(source: &str) -> ParseResult<TestConfig> {
        let mut config = TestConfig { scenes: Vec::new(), tasks: Vec::new(), trials: 20, timeout: 10.0, wait: 0.5 };
        for node in parser::parse(source)? {
            let AstKind::Form { head, args } = &node.kind else {
                return Err(error(&node, "expected a form"));
            };
            match (head.as_str(), args.as_slice()) {
                ("scenes", paths) if !paths.is_empty() => {
                    config.scenes = paths.iter().map(string).collect::<ParseResult<_>>()?;
                }
                ("trials", [n]) => config.trials = number(n)?.max(1.0) as u32,
                ("timeout", [secs]) => config.timeout = number(secs)?,
                ("wait", [secs]) => config.wait = number(secs)?,
                ("task", [name, expect]) => {
                    let AstKind::Form { head: expect_name, args: expect_args } = &expect.kind else {
                        return Err(error(expect, "expected (name value...)"));
                    };
                    config.tasks.push(Task {
                        name: string(name)?,
                        expect_name: expect_name.clone(),
                        expect_args: expect_args.iter().map(number).collect::<ParseResult<_>>()?,
                    });
                }
                _ => return Err(error(&node, format!("unknown or malformed form `{}`", head))),
            }
        }
        if config.scenes.is_empty() || config.tasks.is_empty() {
            return Err(ParseError {
                message: "need (scenes ...) and at least one (task ...)".to_string(),
                span: parser::Span { start: 0, end: 0 },
            });
        }
        Ok(config)
    }
}

/// 乱数（SplitMix64）。シードをログに残せば出題順を再現できる
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// ソースが同じかどうかをログで見分けるためのハッシュ（FNV-1a。Rust のバージョンに依存しない）
fn hash(text: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.bytes() {
        h ^= byte as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{:016x}", h)
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Phase {
    /// 試行の前の待ち時間。シーンは初期状態で表示しておく
    Waiting { scene: usize, until: u64 },
    Running { scene: usize, task: usize, shown: u64 },
    Finished,
}

pub struct TestSession {
    config: TestConfig,
    players: Vec<ScenePlayer>,
    rng: Rng,
    clock: FixedClock,
    /// 計測全体で 1 本の時間軸（ログの tick はこれ）
    tick: u64,
    /// 始めた試行の数
    started_trials: u32,
    first_scene: usize,
    phase: Phase,
    pending_inputs: Vec<InputEvent>,
    params: Vec<Vec<(String, f32)>>,
    log: Box<dyn Write>,
    log_path: String,
}

impl TestSession {
    /// 設定ファイルを読み、`logs/` にログファイルを作って計測を始める
    pub fn start() -> anyhow::Result<TestSession> {
        let source = assets::read_to_string(CONFIG_PATH)?;
        let config = TestConfig::compile(&source).map_err(|e| {
            let (line, col) = parser::line_col(&source, e.span.start);
            anyhow::anyhow!("{}:{}:{}: {}", CONFIG_PATH, line, col, e.message)
        })?;
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        let log_path = format!("logs/session-{}.log", utc_compact(now.as_secs()));
        let file = assets::create_log(&log_path)?;
        let seed = now.as_nanos() as u64;
        let header = format!("(started {}) ", quote(&utc_readable(now.as_secs())));
        Ok(TestSession::new(config, seed, Box::new(file), log_path, &header))
    }

    /// `log` に書き込む計測を作る（テストでは任意の書き込み先を渡す）
    pub fn new(config: TestConfig, seed: u64, log: Box<dyn Write>, log_path: String, header: &str) -> TestSession {
        let players: Vec<ScenePlayer> = config.scenes.iter().map(|path| ScenePlayer::new(path)).collect();
        let params = players.iter().map(|p| p.param_values()).collect();
        let mut rng = Rng(seed);
        let first_scene = rng.below(players.len());
        let mut session = TestSession {
            config,
            players,
            rng,
            clock: FixedClock::new(),
            tick: 0,
            started_trials: 0,
            first_scene,
            phase: Phase::Finished,
            pending_inputs: Vec::new(),
            params,
            log,
            log_path,
        };
        session.write(format!(
            "(session {}(seed {}) (ticks_per_second {}) (config {}))",
            header,
            seed,
            TICKS_PER_SECOND,
            quote(CONFIG_PATH)
        ));
        for i in 0..session.players.len() {
            let player = &session.players[i];
            let params: Vec<String> = player.param_values().iter().map(|(n, v)| format!("({} {})", n, v)).collect();
            let line = format!(
                "(scene {} (path {}) (source_hash {}) (params {}))",
                i,
                quote(player.path()),
                quote(&hash(player.source())),
                params.join(" ")
            );
            session.write(line);
        }
        session.begin_wait(first_scene);
        session
    }

    pub fn log_path(&self) -> &str {
        &self.log_path
    }

    pub fn is_finished(&self) -> bool {
        self.phase == Phase::Finished
    }

    fn write(&mut self, line: String) {
        if let Err(e) = writeln!(self.log, "{}", line).and_then(|_| self.log.flush()) {
            log::error!("could not write {}: {}", self.log_path, e);
        }
    }

    fn ticks(&self, seconds: f32) -> u64 {
        (seconds * TICKS_PER_SECOND as f32).round().max(0.0) as u64
    }

    fn begin_wait(&mut self, scene: usize) {
        self.players[scene].reset();
        let until = self.tick + self.ticks(self.config.wait);
        self.phase = Phase::Waiting { scene, until };
    }

    pub fn queue_inputs(&mut self, inputs: &[InputEvent]) {
        self.pending_inputs.extend_from_slice(inputs);
    }

    /// 実時間ぶん進める。シーンファイルの変更もここで拾い、パラメータが変わっていればログに残す
    pub fn update(&mut self, logical_size: [f32; 2]) {
        for i in 0..self.players.len() {
            self.players[i].poll_file();
            let values = self.players[i].param_values();
            if values != self.params[i] {
                let changed: Vec<String> = values
                    .iter()
                    .filter(|v| !self.params[i].contains(v))
                    .map(|(name, value)| format!("(param {} {} {} {})", self.tick, i, name, value))
                    .collect();
                for line in changed {
                    self.write(line);
                }
                self.params[i] = values;
            }
        }
        for _ in 0..self.clock.ticks_due() {
            self.step_tick(logical_size);
        }
    }

    /// 1 tick 進める（決定的。テストはこれを直接呼ぶ）
    pub fn step_tick(&mut self, logical_size: [f32; 2]) {
        self.tick += 1;
        let tick = self.tick;
        let inputs = std::mem::take(&mut self.pending_inputs);
        match self.phase {
            // 待っている間の入力は捨てる（試行の条件をそろえるため）
            Phase::Waiting { scene, until } => {
                if tick >= until {
                    let task = self.rng.below(self.config.tasks.len());
                    self.started_trials += 1;
                    let t = &self.config.tasks[task];
                    let expect: Vec<String> = t.expect_args.iter().map(|a| a.to_string()).collect();
                    let line = format!(
                        "(trial {} (scene {}) (task {}) (expect {} {}) (shown {}))",
                        self.started_trials,
                        scene,
                        quote(&t.name),
                        t.expect_name,
                        expect.join(" "),
                        tick
                    );
                    self.write(line);
                    self.phase = Phase::Running { scene, task, shown: tick };
                }
            }
            Phase::Running { scene, task, shown } => {
                let mut lines = Vec::new();
                for input in &inputs {
                    let edge = if input.edge == Edge::Press { "press" } else { "release" };
                    lines.push(format!("(input {} {} {})", tick, edge, input.action.name()));
                }
                let expected = &self.config.tasks[task];
                let mut result = None;
                self.players[scene].step_tick(&inputs, logical_size, &mut |event| match event {
                    StepEvent::Transition { from, to, .. } => lines.push(format!("(transition {} {} {})", tick, from, to)),
                    StepEvent::Ignored { input, .. } => {
                        lines.push(format!("(ignored {} press {})", tick, input.action.name()))
                    }
                    StepEvent::Emit { name, args } => {
                        let values: Vec<String> = args.iter().map(|a| a.to_string()).collect();
                        lines.push(format!("(emit {} {} {})", tick, name, values.join(" ")));
                        if result.is_none() && name == expected.expect_name {
                            result = Some(if args == expected.expect_args {
                                TrialResult::Complete
                            } else {
                                TrialResult::Wrong
                            });
                        }
                    }
                    StepEvent::Internal { .. } => {}
                });
                if result.is_none() && tick - shown >= self.ticks(self.config.timeout) {
                    result = Some(TrialResult::Timeout);
                }
                for line in lines {
                    self.write(line);
                }
                if let Some(result) = result {
                    self.end_trial(result);
                    if self.started_trials >= self.config.trials {
                        self.finish();
                    } else {
                        let next = (self.first_scene + self.started_trials as usize) % self.players.len();
                        self.begin_wait(next);
                    }
                }
            }
            Phase::Finished => {}
        }
    }

    fn end_trial(&mut self, result: TrialResult) {
        let line = format!("(trial_end {} {} {})", self.started_trials, result.name(), self.tick);
        self.write(line);
    }

    /// 計測を終える。試行の途中なら中断として記録する
    pub fn finish(&mut self) {
        if self.phase == Phase::Finished {
            return;
        }
        if matches!(self.phase, Phase::Running { .. }) {
            self.end_trial(TrialResult::Aborted);
        }
        let line = format!("(session_end {})", self.tick);
        self.write(line);
        self.phase = Phase::Finished;
        log::info!("session finished: {}", self.log_path);
    }

    /// 今の試行のシーン番号と課題（テスト・表示用）
    pub fn current(&self) -> Option<(usize, Option<&Task>)> {
        match self.phase {
            Phase::Waiting { scene, .. } => Some((scene, None)),
            Phase::Running { scene, task, .. } => Some((scene, Some(&self.config.tasks[task]))),
            Phase::Finished => None,
        }
    }

    /// 今のシーンを描き、課題と進み具合を上部に重ねる
    pub fn build(&mut self, logical_size: [f32; 2], rects: &mut Vec<RectInstance>, labels: &mut Vec<Label>) -> Option<[f32; 4]> {
        let (scene, task) = self.current()?;
        let task_name = task.map(|t| t.name.clone());
        let background = self.players[scene].build(logical_size, rects);

        rects.push(RectInstance {
            position: [0.0, 0.0],
            size: [logical_size[0], 72.0],
            color: srgb8(0, 0, 0, 170),
            rotation: 0.0,
        });
        if let Some(name) = task_name {
            labels.push(Label { text: name, position: [24.0, 12.0], size: 40.0, color: [255, 255, 255, 255] });
        }
        labels.push(Label {
            text: format!(
                "計測中 {}/{}   {}   T で中断",
                self.started_trials.max(1),
                self.config.trials,
                self.players[scene].path()
            ),
            position: [logical_size[0] - 330.0, 8.0],
            size: 12.0,
            color: [200, 200, 210, 255],
        });
        background
    }
}

/// UNIX 時間（秒）→ UTC の (年, 月, 日, 時, 分, 秒)
fn utc(secs: u64) -> (i64, u32, u32, u64, u64, u64) {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Howard Hinnant の civil_from_days
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + if month <= 2 { 1 } else { 0 };
    (year, month, day, rem / 3_600, rem % 3_600 / 60, rem % 60)
}

fn utc_compact(secs: u64) -> String {
    let (y, mo, d, h, mi, s) = utc(secs);
    format!("{:04}{:02}{:02}-{:02}{:02}{:02}", y, mo, d, h, mi, s)
}

fn utc_readable(secs: u64) -> String {
    let (y, mo, d, h, mi, s) = utc(secs);
    format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC", y, mo, d, h, mi, s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis;
    use engine::input::Action;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// テスト用の書き込み先（中身を後から読める）
    #[derive(Clone, Default)]
    struct SharedBuffer(Rc<RefCell<Vec<u8>>>);

    impl Write for SharedBuffer {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.borrow_mut().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    const SIZE: [f32; 2] = [800.0, 600.0];

    fn press(action: Action) -> InputEvent {
        InputEvent { action, edge: Edge::Press }
    }

    fn release(action: Action) -> InputEvent {
        InputEvent { action, edge: Edge::Release }
    }

    #[test]
    fn compiles_config() {
        let config = TestConfig::compile(include_str!("../ui/tasks.ui")).unwrap();
        assert_eq!(config.scenes, vec!["ui/menu_a.ui", "ui/menu_b.ui"]);
        assert_eq!(config.tasks.len(), 4);
        assert_eq!(config.tasks[1].expect_args, vec![1.0]);
        assert!(TestConfig::compile("(task \"x\" (opened 1))").is_err()); // scenes がない
    }

    #[test]
    fn utc_conversion() {
        assert_eq!(utc_readable(0), "1970-01-01 00:00:00 UTC");
        assert_eq!(utc_compact(1_772_000_000), "20260225-061320");
    }

    /// 両方式で課題どおりに操作し、ログから集計できること
    #[test]
    fn runs_trials_on_both_scenes_and_logs_them() {
        let mut config = TestConfig::compile(include_str!("../ui/tasks.ui")).unwrap();
        config.trials = 8;
        config.wait = 0.1;
        let buffer = SharedBuffer::default();
        let mut session = TestSession::new(config, 42, Box::new(buffer.clone()), "test.log".into(), "");

        let mut guard = 0;
        while !session.is_finished() {
            guard += 1;
            assert!(guard < 10_000, "session did not finish");
            let Some((scene, Some(task))) = session.current() else {
                session.step_tick(SIZE);
                continue;
            };
            let target = task.expect_args[0] as usize;
            // 方式A（0）: 押したまま方向キー / 方式B（1）: 開いて下で選んで決定
            let script: Vec<InputEvent> = if session.config.scenes[scene].ends_with("menu_a.ui") {
                let dir = [Action::Up, Action::Right, Action::Down, Action::Left][target];
                vec![press(Action::Menu), press(dir), release(Action::Menu)]
            } else {
                let mut s = vec![press(Action::Menu)];
                s.extend(std::iter::repeat_n(press(Action::Down), target));
                s.push(press(Action::Confirm));
                s
            };
            for input in script {
                session.queue_inputs(&[input]);
                session.step_tick(SIZE);
            }
            // 課題を出してからの試行が終わるまで進める
            while matches!(session.phase, Phase::Running { .. }) {
                session.step_tick(SIZE);
            }
        }

        let log = String::from_utf8(buffer.0.borrow().clone()).unwrap();
        let report = analysis::analyze(&log).unwrap();
        assert_eq!(report.trials.len(), 8);
        assert!(report.trials.iter().all(|t| t.result == Some(TrialResult::Complete)), "{}", log);
        // 交互に出題される
        assert!(report.trials.windows(2).all(|w| w[0].scene != w[1].scene));
        for stats in &report.scenes {
            assert_eq!((stats.trials, stats.complete), (2, 2));
        }
    }

    #[test]
    fn wrong_screen_and_timeout_end_the_trial() {
        let mut config = TestConfig::compile(include_str!("../ui/tasks.ui")).unwrap();
        config.scenes = vec!["ui/menu_b.ui".into()];
        config.trials = 2;
        config.wait = 0.0;
        config.timeout = 0.5;
        let buffer = SharedBuffer::default();
        let mut session = TestSession::new(config, 7, Box::new(buffer.clone()), "test.log".into(), "");
        session.step_tick(SIZE); // 課題を表示

        // 期待と違う画面を開く
        let target = session.current().unwrap().1.unwrap().expect_args[0] as usize;
        let downs = (target + 1) % 4;
        session.queue_inputs(&[press(Action::Menu)]);
        session.step_tick(SIZE);
        for _ in 0..downs {
            session.queue_inputs(&[press(Action::Down)]);
            session.step_tick(SIZE);
        }
        session.queue_inputs(&[press(Action::Confirm)]);
        session.step_tick(SIZE);

        // 2 回目は何もしないで時間切れ
        for _ in 0..100 {
            session.step_tick(SIZE);
        }
        assert!(session.is_finished());

        let log = String::from_utf8(buffer.0.borrow().clone()).unwrap();
        let report = analysis::analyze(&log).unwrap();
        let results: Vec<_> = report.trials.iter().map(|t| t.result).collect();
        assert_eq!(results, vec![Some(TrialResult::Wrong), Some(TrialResult::Timeout)], "{}", log);
    }
}
