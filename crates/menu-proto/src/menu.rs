//! メニュー試作: 方式A・方式B のシーンを 1 つずつ動かすウィンドウと、インスペクタのウィンドウ。
//! T キーで計測モード（`session.rs`）。

use dsl::parser;
use engine::input::InputEvent;
use engine::renderer::srgb8;
use engine::winit::{keyboard::KeyCode, window::WindowId};
use engine::{Frame, Gpu, Pointer, PointerKind, Product, WindowSpec, assets};
use uikit::ScenePlayer;
use uikit::inspector::{self, Inspector, SceneSummary};

use crate::analysis;
use crate::session::TestSession;

/// ウィンドウ i に読み込む UI シーン
const SCENE_PATHS: [&str; 2] = ["ui/menu_a.ui", "ui/menu_b.ui"];

/// 開くウィンドウ。シーンの数だけ並べ、最後がインスペクタ（wasm では先頭だけ）
pub fn window_specs() -> Vec<WindowSpec> {
    vec![
        WindowSpec { title: "方式A".into(), size: None },
        WindowSpec { title: "方式B".into(), size: None },
        WindowSpec { title: "インスペクタ".into(), size: Some([420.0, 640.0]) },
    ]
}

/// シーンを動かす 1 ウィンドウ
struct SceneWindow {
    id: WindowId,
    scene: ScenePlayer,
    /// 計測中なら Some。計測中はシーンの代わりにこちらを動かして描く
    session: Option<TestSession>,
}

pub struct MenuProto {
    /// 並び順はウィンドウを開いた順（インスペクタの表示順になる）
    scenes: Vec<SceneWindow>,
    inspector: Inspector,
    /// パラメータを並べるウィンドウ（シーンは動かさない）
    inspector_window: Option<WindowId>,
    /// 直近の計測の集計（インスペクタに出す行）
    report: Vec<String>,
}

impl MenuProto {
    /// `windows` は `window_specs` の順。シーンより多ければ次の 1 つをインスペクタにする
    pub fn new(_gpu: &Gpu, windows: &[WindowId]) -> anyhow::Result<Self> {
        let scene_count = windows.len().min(SCENE_PATHS.len());
        let scenes = windows[..scene_count]
            .iter()
            .zip(SCENE_PATHS)
            .map(|(&id, path)| SceneWindow { id, scene: ScenePlayer::new(path), session: None })
            .collect();
        Ok(Self {
            scenes,
            inspector: Inspector::default(),
            inspector_window: windows.get(scene_count).copied(),
            report: Vec::new(),
        })
    }

    fn scene_window(&mut self, id: WindowId) -> Option<&mut SceneWindow> {
        self.scenes.iter_mut().find(|w| w.id == id)
    }

    fn scene_summaries(scenes: &[SceneWindow]) -> Vec<SceneSummary<'_>> {
        scenes
            .iter()
            .map(|w| SceneSummary {
                window: w.id,
                path: w.scene.path(),
                state: w.scene.state_name(),
                error: w.scene.last_error(),
                params: w.scene.params(),
            })
            .collect()
    }

    /// T キー: そのウィンドウで計測を始める。計測中ならやめる
    fn toggle_session(&mut self, window_id: WindowId) {
        let Some(window) = self.scenes.iter_mut().find(|w| w.id == window_id) else {
            return;
        };
        match &mut window.session {
            Some(session) if !session.is_finished() => session.finish(),
            _ => match TestSession::start() {
                Ok(session) => {
                    log::info!("session started: {}", session.log_path());
                    self.report = vec![format!("計測中: {}", session.log_path())];
                    window.session = Some(session);
                }
                Err(e) => {
                    log::error!("could not start session: {:#}", e);
                    self.report = vec![format!("計測を始められません: {:#}", e)];
                }
            },
        }
        self.collect_finished_sessions();
    }

    /// 終わった計測のログを読み込んで集計する（毎フレーム呼んでよい）
    fn collect_finished_sessions(&mut self) {
        for window in &mut self.scenes {
            if !window.session.as_ref().is_some_and(|s| s.is_finished()) {
                continue;
            }
            let path = window.session.take().unwrap().log_path().to_string();
            let result = assets::read_to_string(&path).and_then(|log| {
                analysis::analyze(&log).map_err(|e| {
                    let (line, col) = parser::line_col(&log, e.span.start);
                    anyhow::anyhow!("{}:{}:{}: {}", path, line, col, e.message)
                })
            });
            self.report = match result {
                Ok(report) => std::iter::once(format!("計測結果: {}", path)).chain(report.lines()).collect(),
                Err(e) => vec![format!("集計できません: {:#}", e)],
            };
        }
    }

    /// インスペクタでカーソルが動いた。ドラッグ中ならパラメータの値を変える（メモリ上だけ）
    fn inspector_drag(&mut self, cursor: [f32; 2], width: f32) {
        let Some((window_id, name)) = self.inspector.dragging().map(|(w, n)| (w, n.to_string())) else {
            return;
        };
        let inspector = &self.inspector;
        let Some(window) = self.scenes.iter_mut().find(|w| w.id == window_id) else {
            return;
        };
        let scene = &mut window.scene;
        if let Some(value) = scene.params().iter().find(|p| p.name == name).map(|p| inspector.value_at(p, cursor[0], width)) {
            scene.set_param(&name, value);
        }
    }
}

impl Product for MenuProto {
    fn key(&mut self, window: WindowId, code: KeyCode, pressed: bool, repeat: bool) {
        if code == KeyCode::KeyT && pressed && !repeat {
            self.toggle_session(window);
        }
    }

    /// 計測中なら計測に、そうでなければシーンに入力を渡す
    fn inputs(&mut self, window: WindowId, inputs: &[InputEvent]) {
        let Some(window) = self.scene_window(window) else {
            return;
        };
        if let Some(session) = window.session.as_mut().filter(|s| !s.is_finished()) {
            session.queue_inputs(inputs);
        } else {
            window.scene.queue_inputs(inputs);
        }
    }

    /// インスペクタでのマウス。押したらドラッグ開始、動かしたら値を変え、離したらソースに書き戻す
    fn pointer(&mut self, pointer: Pointer) {
        if Some(pointer.window) != self.inspector_window {
            return;
        }
        let width = pointer.window_size[0];
        match pointer.kind {
            PointerKind::Moved => self.inspector_drag(pointer.position, width),
            PointerKind::Down => {
                if self.inspector.press(&Self::scene_summaries(&self.scenes), pointer.position).is_some() {
                    // 範囲のあるスライダーは、押した位置の値にすぐ合わせる
                    self.inspector_drag(pointer.position, width);
                }
            }
            PointerKind::Up => {
                if let Some((window_id, name)) = self.inspector.release() {
                    if let Some(window) = self.scene_window(window_id) {
                        let scene = &mut window.scene;
                        if let Some(value) = scene.params().iter().find(|p| p.name == name).map(|p| p.value) {
                            scene.write_param(&name, value);
                        }
                    }
                }
            }
        }
    }

    fn reload(&mut self, _gpu: &Gpu) {
        for window in &mut self.scenes {
            window.scene.reload();
        }
    }

    fn update(&mut self, _gpu: &Gpu, frame: &mut Frame) {
        self.collect_finished_sessions();
        let size = frame.logical_size;
        if Some(frame.window) == self.inspector_window {
            self.inspector.build(&Self::scene_summaries(&self.scenes), &self.report, size[0], frame.rects, frame.labels);
            let [r, g, b, _] = inspector::BACKGROUND;
            frame.background = Some(srgb8(r, g, b, 255));
            return;
        }
        let Some(window) = self.scene_window(frame.window) else {
            return;
        };
        if let Some(session) = window.session.as_mut().filter(|s| !s.is_finished()) {
            session.update(size);
            frame.background = session.build(size, frame.rects, frame.labels);
        } else {
            let scene = &mut window.scene;
            scene.poll_file();
            scene.advance(size);
            frame.background = scene.build(size, frame.rects);
        }
    }

    /// シーンを動かすウィンドウが残っていなければ終了する
    fn window_closed(&mut self, window: WindowId) -> bool {
        self.scenes.retain(|w| w.id != window);
        if self.inspector_window == Some(window) {
            self.inspector_window = None;
        }
        self.scenes.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use engine::input::{Action, Edge, InputEvent};
    use uikit::scene::{Runtime, Scene};

    const SIZE: [f32; 2] = [800.0, 600.0];

    fn compile(source: &str) -> Scene {
        Scene::compile(source).unwrap_or_else(|e| panic!("{} at {:?}", e, dsl::parser::line_col(source, e.span.start)))
    }

    fn press(action: Action) -> InputEvent {
        InputEvent { action, edge: Edge::Press }
    }

    fn release(action: Action) -> InputEvent {
        InputEvent { action, edge: Edge::Release }
    }

    fn step(scene: &Scene, runtime: &mut Runtime, tick: u64, inputs: &[InputEvent]) {
        scene.step(runtime, tick, inputs, SIZE, &mut Vec::new(), &mut |_| {});
    }

    fn state_name<'a>(scene: &'a Scene, runtime: &Runtime) -> &'a str {
        scene.state_name(runtime).unwrap()
    }

    /// 同梱のシーンファイルが壊れていないこと、主な操作の流れが想定どおりであること
    #[test]
    fn bundled_menu_a_hold_and_direction() {
        let scene = compile(include_str!("../ui/menu_a.ui"));
        let mut rt = scene.adopt(None, 0);
        // 押してすぐ離すと何も開かない
        step(&scene, &mut rt, 1, &[press(Action::Menu)]);
        step(&scene, &mut rt, 2, &[release(Action::Menu)]);
        assert_eq!(state_name(&scene, &rt), "closed");
        // 押したまま右 → 画面1
        step(&scene, &mut rt, 3, &[press(Action::Menu)]);
        step(&scene, &mut rt, 4, &[press(Action::Right)]);
        step(&scene, &mut rt, 5, &[release(Action::Menu)]);
        assert_eq!((state_name(&scene, &rt), rt.vars()[0]), ("page", 1.0));
        step(&scene, &mut rt, 6, &[press(Action::Cancel)]);
        assert_eq!(state_name(&scene, &rt), "closed");
    }

    #[test]
    fn bundled_menu_b_select_and_confirm() {
        let scene = compile(include_str!("../ui/menu_b.ui"));
        let mut rt = scene.adopt(None, 0);
        step(&scene, &mut rt, 1, &[press(Action::Menu), release(Action::Menu)]);
        step(&scene, &mut rt, 2, &[press(Action::Up)]); // 0 → 3（折り返し）
        step(&scene, &mut rt, 3, &[press(Action::Up), press(Action::Confirm)]);
        assert_eq!(state_name(&scene, &rt), "page");
        assert_eq!(rt.vars(), vec![2.0, 2.0]); // cursor, screen
        step(&scene, &mut rt, 4, &[press(Action::Cancel)]);
        assert_eq!(state_name(&scene, &rt), "open");
    }

}
