//! パラメータのインスペクタ（別ウィンドウ）。
//! 各シーンの `defparam` をスライダーとして並べる。ドラッグ中はメモリ上の値だけ変え、離したらソースに書き戻す。
//! 座標はすべて論理ピクセル。

use engine::renderer::{RectInstance, srgb8};
use engine::text::Label;
use engine::winit::window::WindowId;

use crate::scene::{Param, ParamKind};

/// インスペクタに並べる1シーン分の情報
pub struct SceneSummary<'a> {
    pub window: WindowId,
    pub path: &'a str,
    pub state: Option<&'a str>,
    pub error: Option<&'a str>,
    pub params: &'a [Param],
}

const PADDING: f32 = 16.0;
const HEADER_H: f32 = 30.0;
const ERROR_H: f32 = 22.0;
const ROW_H: f32 = 44.0;
const SCENE_GAP: f32 = 18.0;
/// 行の上端からスライダーのつまみの通り道（トラック）までの距離
const TRACK_Y: f32 = 26.0;
const TRACK_H: f32 = 4.0;

pub const BACKGROUND: [u8; 4] = [28, 28, 34, 255];

enum RowKind {
    Header,
    Error,
    Param(usize),
    /// 計測結果の i 行目
    Report(usize),
}

struct Row {
    scene: usize,
    kind: RowKind,
    y: f32,
}

const REPORT_H: f32 = 20.0;

/// 上から順に行を並べる。描画と当たり判定で同じものを使う。計測結果はいちばん下
fn layout(scenes: &[SceneSummary], report_len: usize) -> Vec<Row> {
    let mut rows = Vec::new();
    let mut y = PADDING;
    for (scene, summary) in scenes.iter().enumerate() {
        rows.push(Row { scene, kind: RowKind::Header, y });
        y += HEADER_H;
        if summary.error.is_some() {
            rows.push(Row { scene, kind: RowKind::Error, y });
            y += ERROR_H;
        }
        for i in 0..summary.params.len() {
            rows.push(Row { scene, kind: RowKind::Param(i), y });
            y += ROW_H;
        }
        y += SCENE_GAP;
    }
    for i in 0..report_len {
        rows.push(Row { scene: 0, kind: RowKind::Report(i), y });
        y += REPORT_H;
    }
    rows
}

fn track_width(width: f32) -> f32 {
    (width - PADDING * 2.0).max(1.0)
}

/// 範囲のあるパラメータの、トラック上の位置（0–1）
fn fraction(param: &Param, value: f32) -> Option<f32> {
    match (param.min, param.max) {
        (Some(min), Some(max)) if max > min => Some(((value - min) / (max - min)).clamp(0.0, 1.0)),
        _ => None,
    }
}

struct Drag {
    window: WindowId,
    param: String,
    start_value: f32,
    start_x: f32,
}

#[derive(Default)]
pub struct Inspector {
    drag: Option<Drag>,
}

impl Inspector {
    pub fn build(
        &self,
        scenes: &[SceneSummary],
        report: &[String],
        width: f32,
        rects: &mut Vec<RectInstance>,
        labels: &mut Vec<Label>,
    ) {
        let rect = |x: f32, y: f32, w: f32, h: f32, color: [f32; 4]| RectInstance {
            position: [x, y],
            size: [w, h],
            color,
            rotation: 0.0,
        };
        let track_w = track_width(width);

        for row in layout(scenes, report.len()) {
            if let RowKind::Report(i) = row.kind {
                labels.push(Label {
                    text: report[i].clone(),
                    position: [PADDING, row.y],
                    size: if i == 0 { 14.0 } else { 12.0 },
                    color: if i == 0 { [255, 255, 255, 255] } else { [190, 220, 255, 255] },
                });
                continue;
            }
            let summary = &scenes[row.scene];
            match row.kind {
                RowKind::Header => labels.push(Label {
                    text: format!("{}   state: {}", summary.path, summary.state.unwrap_or("-")),
                    position: [PADDING, row.y + 6.0],
                    size: 15.0,
                    color: [255, 255, 255, 255],
                }),
                RowKind::Error => labels.push(Label {
                    text: summary.error.unwrap_or_default().to_string(),
                    position: [PADDING, row.y + 2.0],
                    size: 12.0,
                    color: [255, 110, 110, 255],
                }),
                RowKind::Param(i) => {
                    let param = &summary.params[i];
                    if self.is_dragging(summary.window, &param.name) {
                        rects.push(rect(PADDING / 2.0, row.y, width - PADDING, ROW_H - 4.0, srgb8(255, 255, 255, 18)));
                    }
                    let track_y = row.y + TRACK_Y;
                    rects.push(rect(PADDING, track_y, track_w, TRACK_H, srgb8(70, 70, 82, 255)));
                    let hint = match fraction(param, param.value) {
                        Some(t) => {
                            rects.push(rect(PADDING, track_y, track_w * t, TRACK_H, srgb8(90, 160, 255, 255)));
                            rects.push(rect(PADDING + track_w * t - 5.0, track_y - 6.0, 10.0, 16.0, srgb8(235, 235, 245, 255)));
                            ""
                        }
                        None => "   (左右にドラッグで増減)",
                    };
                    labels.push(Label {
                        text: format!("{}   {}{}", param.name, param.format_value(param.value), hint),
                        position: [PADDING, row.y + 4.0],
                        size: 13.0,
                        color: [210, 210, 220, 255],
                    });
                }
                RowKind::Report(_) => {}
            }
        }
    }

    fn is_dragging(&self, window: WindowId, name: &str) -> bool {
        self.drag.as_ref().is_some_and(|d| d.window == window && d.param == name)
    }

    /// クリックした行のパラメータでドラッグを始める。始めたら (ウィンドウ, 名前) を返す
    pub fn press(&mut self, scenes: &[SceneSummary], cursor: [f32; 2]) -> Option<(WindowId, String)> {
        let row = layout(scenes, 0)
            .into_iter()
            .find(|row| matches!(row.kind, RowKind::Param(_)) && (row.y..row.y + ROW_H).contains(&cursor[1]))?;
        let RowKind::Param(i) = row.kind else {
            return None;
        };
        let summary = &scenes[row.scene];
        let param = &summary.params[i];
        self.drag = Some(Drag {
            window: summary.window,
            param: param.name.clone(),
            start_value: param.value,
            start_x: cursor[0],
        });
        Some((summary.window, param.name.clone()))
    }

    pub fn dragging(&self) -> Option<(WindowId, &str)> {
        self.drag.as_ref().map(|d| (d.window, d.param.as_str()))
    }

    /// カーソル位置に対応する値。範囲があればトラック上の位置、なければドラッグ開始からの移動量で決める
    pub fn value_at(&self, param: &Param, cursor_x: f32, width: f32) -> f32 {
        let value = match fraction(param, param.value) {
            Some(_) => {
                let (min, max) = (param.min.unwrap(), param.max.unwrap());
                let t = ((cursor_x - PADDING) / track_width(width)).clamp(0.0, 1.0);
                min + t * (max - min)
            }
            None => {
                let Some(drag) = &self.drag else {
                    return param.value;
                };
                // 1 ピクセルあたりの変化量
                let unit = param.step.unwrap_or(match param.kind {
                    ParamKind::Int => 0.25,
                    ParamKind::Float => drag.start_value.abs().max(1.0) / 200.0,
                });
                drag.start_value + (cursor_x - drag.start_x) * unit
            }
        };
        param.snap(value)
    }

    /// ドラッグを終える。終えたら (ウィンドウ, 名前) を返す
    pub fn release(&mut self) -> Option<(WindowId, String)> {
        self.drag.take().map(|d| (d.window, d.param))
    }
}
