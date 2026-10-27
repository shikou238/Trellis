//! 入力の2層化。
//! 生の入力（キーの押下・解放）を、抽象的な操作（メニュー、上、決定など）の押下・解放に変換する。
//! シーンは抽象的な操作だけを見るので、キー配置やデバイス（将来はパッド・タッチ）を差し替えられる。

use winit::keyboard::KeyCode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Menu,
    Up,
    Down,
    Left,
    Right,
    Confirm,
    Cancel,
}

impl Action {
    pub const ALL: [Action; 7] = [
        Action::Menu,
        Action::Up,
        Action::Down,
        Action::Left,
        Action::Right,
        Action::Confirm,
        Action::Cancel,
    ];

    /// DSL で使う名前
    pub fn name(self) -> &'static str {
        match self {
            Action::Menu => "menu",
            Action::Up => "up",
            Action::Down => "down",
            Action::Left => "left",
            Action::Right => "right",
            Action::Confirm => "confirm",
            Action::Cancel => "cancel",
        }
    }

    pub fn from_name(name: &str) -> Option<Action> {
        Action::ALL.into_iter().find(|a| a.name() == name)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Press,
    Release,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputEvent {
    pub action: Action,
    pub edge: Edge,
}

/// キー配置。カメラ操作（WASD）とは重ならないようにしている
fn action_for_key(code: KeyCode) -> Option<Action> {
    Some(match code {
        KeyCode::Space => Action::Menu,
        KeyCode::ArrowUp => Action::Up,
        KeyCode::ArrowDown => Action::Down,
        KeyCode::ArrowLeft => Action::Left,
        KeyCode::ArrowRight => Action::Right,
        KeyCode::Enter | KeyCode::KeyZ => Action::Confirm,
        KeyCode::Backspace | KeyCode::KeyX => Action::Cancel,
        _ => return None,
    })
}

/// 押されているキーを追跡し、操作の押下・解放イベントを作る。
/// 同じ操作に複数のキーが割り当てられていても、操作としては「最初に押された・最後に離された」ときだけイベントになる
#[derive(Default)]
pub struct InputMapper {
    held_keys: Vec<KeyCode>,
}

impl InputMapper {
    pub fn key(&mut self, code: KeyCode, pressed: bool, repeat: bool, out: &mut Vec<InputEvent>) {
        let Some(action) = action_for_key(code) else {
            return;
        };
        // OS のキーリピートは無視する（押しっぱなしは解放が来るまで1回の押下）
        if repeat {
            return;
        }
        let was_held = self.is_held(action);
        if pressed {
            if !self.held_keys.contains(&code) {
                self.held_keys.push(code);
            }
        } else {
            self.held_keys.retain(|k| *k != code);
        }
        match (was_held, self.is_held(action)) {
            (false, true) => out.push(InputEvent { action, edge: Edge::Press }),
            (true, false) => out.push(InputEvent { action, edge: Edge::Release }),
            _ => {}
        }
    }

    /// フォーカスを失ったときなど、解放が届かないキーをすべて離したことにする
    pub fn release_all(&mut self, out: &mut Vec<InputEvent>) {
        for action in Action::ALL {
            if self.is_held(action) {
                out.push(InputEvent { action, edge: Edge::Release });
            }
        }
        self.held_keys.clear();
    }

    fn is_held(&self, action: Action) -> bool {
        self.held_keys.iter().any(|k| action_for_key(*k) == Some(action))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_keys_for_one_action_produce_one_press_and_one_release() {
        let mut mapper = InputMapper::default();
        let mut out = Vec::new();
        mapper.key(KeyCode::Enter, true, false, &mut out);
        mapper.key(KeyCode::KeyZ, true, false, &mut out);
        mapper.key(KeyCode::Enter, false, false, &mut out);
        mapper.key(KeyCode::KeyZ, false, false, &mut out);
        assert_eq!(
            out,
            vec![
                InputEvent { action: Action::Confirm, edge: Edge::Press },
                InputEvent { action: Action::Confirm, edge: Edge::Release },
            ]
        );
    }

    #[test]
    fn ignores_repeat_and_unmapped_keys() {
        let mut mapper = InputMapper::default();
        let mut out = Vec::new();
        mapper.key(KeyCode::Space, true, false, &mut out);
        mapper.key(KeyCode::Space, true, true, &mut out);
        mapper.key(KeyCode::KeyQ, true, false, &mut out);
        assert_eq!(out.len(), 1);

        mapper.release_all(&mut out);
        assert_eq!(out[1], InputEvent { action: Action::Menu, edge: Edge::Release });
    }
}
