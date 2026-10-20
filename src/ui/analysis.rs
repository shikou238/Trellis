//! 計測ログの集計。
//! ログ（S式、1行1イベント）を DSL と同じパーサーで読み、試行ごとの値とシーンごとの統計を出す。
//! 集計はログだけから計算するので、指標を足したり変えたりしても過去のログから計算し直せる。

use crate::parser::{self, AstKind, AstNode, ParseError, ParseResult};

/// シーンごとに最初の何回を練習として集計から外すか
pub const WARMUP_TRIALS: usize = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrialResult {
    Complete,
    /// 期待と違う通知が来た（違う画面を開いたなど）
    Wrong,
    Timeout,
    /// 計測を途中でやめた
    Aborted,
}

impl TrialResult {
    pub fn name(self) -> &'static str {
        match self {
            TrialResult::Complete => "complete",
            TrialResult::Wrong => "wrong",
            TrialResult::Timeout => "timeout",
            TrialResult::Aborted => "aborted",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        [TrialResult::Complete, TrialResult::Wrong, TrialResult::Timeout, TrialResult::Aborted]
            .into_iter()
            .find(|r| r.name() == name)
    }
}

/// 1 試行分の記録（tick 単位）
#[derive(Debug, Clone, PartialEq)]
pub struct TrialRecord {
    pub index: u32,
    pub scene: usize,
    pub task: String,
    /// 課題を表示した tick
    pub shown: u64,
    /// 最初に押した tick
    pub first_press: Option<u64>,
    pub end: Option<u64>,
    pub result: Option<TrialResult>,
    pub presses: u32,
    /// どの遷移にも当てはまらなかった押下
    pub ignored: u32,
    pub cancels: u32,
}

/// シーンごとの統計。時間は秒、練習の試行は除く
#[derive(Debug, Clone, PartialEq)]
pub struct SceneStats {
    pub scene: usize,
    pub path: String,
    pub trials: usize,
    pub complete: usize,
    pub wrong: usize,
    pub timeout: usize,
    /// 完了した試行の中央値: 表示 → 完了
    pub median_total: Option<f32>,
    /// 表示 → 最初の押下（迷っていた時間）
    pub median_think: Option<f32>,
    /// 最初の押下 → 完了（操作そのものの時間）
    pub median_operate: Option<f32>,
    pub mean_presses: Option<f32>,
    pub mean_ignored: Option<f32>,
    pub mean_cancels: Option<f32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    pub trials: Vec<TrialRecord>,
    pub scenes: Vec<SceneStats>,
}

fn error(node: &AstNode, message: &str) -> ParseError {
    ParseError { message: message.to_string(), span: node.span.clone() }
}

fn int(node: &AstNode) -> ParseResult<u64> {
    match node.kind {
        AstKind::Integer(v) if v >= 0 => Ok(v as u64),
        _ => Err(error(node, "expected a non-negative integer")),
    }
}

fn text(node: &AstNode) -> ParseResult<&str> {
    match &node.kind {
        AstKind::String(s) | AstKind::Symbol(s) => Ok(s),
        _ => Err(error(node, "expected a string or name")),
    }
}

/// `(key value...)` の並びから key の最初の値を探す
fn field<'a>(args: &'a [AstNode], key: &str) -> Option<&'a AstNode> {
    args.iter().find_map(|node| match &node.kind {
        AstKind::Form { head, args } if head == key => args.first(),
        _ => None,
    })
}

fn median(values: &mut [f32]) -> Option<f32> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(|a, b| a.total_cmp(b));
    let mid = values.len() / 2;
    Some(if values.len() % 2 == 0 { (values[mid - 1] + values[mid]) / 2.0 } else { values[mid] })
}

fn mean(values: impl Iterator<Item = f32>) -> Option<f32> {
    let (sum, count) = values.fold((0.0, 0), |(s, c), v| (s + v, c + 1));
    (count > 0).then(|| sum / count as f32)
}

pub fn analyze(log: &str) -> ParseResult<Report> {
    let nodes = parser::parse(log)?;
    let mut ticks_per_second = super::TICKS_PER_SECOND as f32;
    let mut scene_paths: Vec<(usize, String)> = Vec::new();
    let mut trials: Vec<TrialRecord> = Vec::new();
    // 終わっていない試行（最後の要素）にだけイベントを数える
    let mut open = false;

    for node in &nodes {
        let AstKind::Form { head, args } = &node.kind else {
            return Err(error(node, "expected a form"));
        };
        match head.as_str() {
            "session" => {
                if let Some(t) = field(args, "ticks_per_second") {
                    ticks_per_second = int(t)? as f32;
                }
            }
            "scene" => {
                let index = int(args.first().ok_or_else(|| error(node, "scene index missing"))?)? as usize;
                let path = field(args, "path").map(text).transpose()?.unwrap_or("?").to_string();
                scene_paths.push((index, path));
            }
            "trial" => {
                let index = int(args.first().ok_or_else(|| error(node, "trial index missing"))?)? as u32;
                let scene = int(field(args, "scene").ok_or_else(|| error(node, "(scene) missing"))?)? as usize;
                let task = text(field(args, "task").ok_or_else(|| error(node, "(task) missing"))?)?.to_string();
                let shown = int(field(args, "shown").ok_or_else(|| error(node, "(shown) missing"))?)?;
                trials.push(TrialRecord {
                    index,
                    scene,
                    task,
                    shown,
                    first_press: None,
                    end: None,
                    result: None,
                    presses: 0,
                    ignored: 0,
                    cancels: 0,
                });
                open = true;
            }
            "input" if open => {
                // (input tick press|release action)
                let [tick, edge, action] = args.as_slice() else {
                    return Err(error(node, "usage: (input tick edge action)"));
                };
                let trial = trials.last_mut().unwrap();
                if text(edge)? == "press" {
                    trial.presses += 1;
                    trial.first_press.get_or_insert(int(tick)?);
                    if text(action)? == "cancel" {
                        trial.cancels += 1;
                    }
                }
            }
            "ignored" if open => trials.last_mut().unwrap().ignored += 1,
            "trial_end" if open => {
                // (trial_end index result tick)
                let [_, result, tick] = args.as_slice() else {
                    return Err(error(node, "usage: (trial_end index result tick)"));
                };
                let trial = trials.last_mut().unwrap();
                trial.result = Some(TrialResult::from_name(text(result)?).ok_or_else(|| error(result, "unknown result"))?);
                trial.end = Some(int(tick)?);
                open = false;
            }
            // transition / emit / param / session_end や試行の外の行は、いまの集計では使わない
            _ => {}
        }
    }

    let seconds = |ticks: u64| ticks as f32 / ticks_per_second;
    let mut scenes = Vec::new();
    for (scene, path) in scene_paths {
        let counted: Vec<&TrialRecord> = trials
            .iter()
            .filter(|t| t.scene == scene && t.result.is_some_and(|r| r != TrialResult::Aborted))
            .skip(WARMUP_TRIALS)
            .collect();
        let complete: Vec<&&TrialRecord> =
            counted.iter().filter(|t| t.result == Some(TrialResult::Complete)).collect();
        let count = |r: TrialResult| counted.iter().filter(|t| t.result == Some(r)).count();
        let mut total: Vec<f32> = complete.iter().map(|t| seconds(t.end.unwrap() - t.shown)).collect();
        let mut think: Vec<f32> = complete.iter().filter_map(|t| Some(seconds(t.first_press? - t.shown))).collect();
        let mut operate: Vec<f32> =
            complete.iter().filter_map(|t| Some(seconds(t.end.unwrap() - t.first_press?))).collect();
        scenes.push(SceneStats {
            scene,
            path,
            trials: counted.len(),
            complete: complete.len(),
            wrong: count(TrialResult::Wrong),
            timeout: count(TrialResult::Timeout),
            median_total: median(&mut total),
            median_think: median(&mut think),
            median_operate: median(&mut operate),
            mean_presses: mean(counted.iter().map(|t| t.presses as f32)),
            mean_ignored: mean(counted.iter().map(|t| t.ignored as f32)),
            mean_cancels: mean(counted.iter().map(|t| t.cancels as f32)),
        });
    }

    Ok(Report { trials, scenes })
}

impl Report {
    /// インスペクタに出す行
    pub fn lines(&self) -> Vec<String> {
        let secs = |v: Option<f32>| v.map_or("-".to_string(), |v| format!("{:.2}s", v));
        let num = |v: Option<f32>| v.map_or("-".to_string(), |v| format!("{:.1}", v));
        let mut lines = vec![format!("（シーンごとに最初の {} 回は練習として除外）", WARMUP_TRIALS)];
        for s in &self.scenes {
            lines.push(format!("{}   {} 回: 完了 {} / 誤操作 {} / 時間切れ {}", s.path, s.trials, s.complete, s.wrong, s.timeout));
            lines.push(format!(
                "   中央値  合計 {} = 迷い {} + 操作 {}",
                secs(s.median_total),
                secs(s.median_think),
                secs(s.median_operate)
            ));
            lines.push(format!(
                "   平均  押下 {} / 無視 {} / キャンセル {}",
                num(s.mean_presses),
                num(s.mean_ignored),
                num(s.mean_cancels)
            ));
        }
        lines
    }
}

/// ログの1行にする文字列リテラル（パーサーが読めるようにエスケープする）
pub fn quote(s: &str) -> String {
    let mut out = String::from("\"");
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOG: &str = r#"
(session (started "2026-10-20 05:03:00 UTC") (seed 1) (ticks_per_second 60) (config "ui/tasks.ui"))
(scene 0 (path "ui/menu_a.ui") (source_hash "x") (params (tilt -9.01)))
(trial 1 (scene 0) (task "赤") (expect opened 0) (shown 100))
(input 110 press menu)
(trial_end 1 complete 120)
(trial 2 (scene 0) (task "赤") (expect opened 0) (shown 200))
(trial_end 2 complete 260)
(trial 3 (scene 0) (task "青") (expect opened 1) (shown 300))
(input 330 press menu)
(ignored 335 press confirm)
(input 335 press confirm)
(input 345 press right)
(emit 345 opened 1)
(trial_end 3 complete 360)
(trial 4 (scene 0) (task "緑") (expect opened 2) (shown 400))
(input 412 press menu)
(input 418 press up)
(trial_end 4 wrong 418)
(trial 5 (scene 0) (task "黄") (expect opened 3) (shown 500))
(input 520 press cancel)
(trial_end 5 timeout 1100)
(param 1150 0 tilt -5)
(trial 6 (scene 0) (task "赤") (expect opened 0) (shown 1200))
(input 1206 press menu)
(session_end 1210)
"#;

    #[test]
    fn analyzes_trials_and_scene_stats() {
        let report = analyze(LOG).unwrap();
        assert_eq!(report.trials.len(), 6);
        let t3 = &report.trials[2];
        assert_eq!((t3.presses, t3.ignored, t3.first_press, t3.end), (3, 1, Some(330), Some(360)));
        assert_eq!(report.trials[5].result, None); // 中断された試行

        let s = &report.scenes[0];
        // 練習の2回と、終わっていない試行は除く
        assert_eq!((s.trials, s.complete, s.wrong, s.timeout), (3, 1, 1, 1));
        assert_eq!(s.median_total, Some(1.0)); // (360 - 300) / 60
        assert_eq!(s.median_think, Some(0.5));
        assert_eq!(s.median_operate, Some(0.5));
        assert_eq!(s.mean_cancels, Some(1.0 / 3.0));
        assert_eq!(report.lines().len(), 4);
    }

    #[test]
    fn quote_round_trips_through_the_parser() {
        let text = "a \"b\" \\ c\n日本語";
        let parsed = parser::parse(&quote(text)).unwrap();
        assert!(matches!(&parsed[0].kind, AstKind::String(s) if s == text));
    }
}
