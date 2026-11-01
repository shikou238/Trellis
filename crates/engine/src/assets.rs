//! ファイル読み込みの窓口。
//! 読み込みはすべてここを通す。将来は同梱素材やネットワーク経由（スマホでのホットリロード）に差し替える。

use std::time::{Duration, Instant, SystemTime};

use anyhow::Context;

pub fn read_to_string(path: &str) -> anyhow::Result<String> {
    std::fs::read_to_string(path).with_context(|| format!("failed to read {}", path))
}

pub fn read_bytes(path: &str) -> anyhow::Result<Vec<u8>> {
    std::fs::read(path).with_context(|| format!("failed to read {}", path))
}

/// ツールからソースへ書き戻すときに使う
pub fn write_string(path: &str, contents: &str) -> anyhow::Result<()> {
    std::fs::write(path, contents).with_context(|| format!("failed to write {}", path))
}

/// 追記用のファイルを新しく作る（親ディレクトリがなければ作る）。計測のログに使う
pub fn create_log(path: &str) -> anyhow::Result<std::fs::File> {
    if let Some(dir) = std::path::Path::new(path).parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("failed to create {}", dir.display()))?;
    }
    std::fs::File::create(path).with_context(|| format!("failed to create {}", path))
}

/// 更新日時。ホットリロードの変更検知に使う。取得できなければ None
pub fn modified(path: &str) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// 更新日時を見に行く間隔。毎フレーム `Watch::changed` を呼んでも、ファイルを見るのはこの間隔ごと
const WATCH_INTERVAL: Duration = Duration::from_millis(200);

/// 素材ファイルの変更を見張る。描き直した絵を R なしで読み直すのに使う
pub struct Watch {
    files: Vec<(String, Option<SystemTime>)>,
    last_poll: Instant,
}

impl Watch {
    /// 今の更新日時を覚える（ここからの変更を知らせる）
    pub fn new(paths: &[&str]) -> Self {
        let files = paths.iter().map(|p| (p.to_string(), modified(p))).collect();
        Self { files, last_poll: Instant::now() }
    }

    /// 前に呼んでから更新日時が変わったファイル（`new` に渡した順）。毎フレーム呼んでよい
    pub fn changed(&mut self) -> Vec<&str> {
        if self.last_poll.elapsed() < WATCH_INTERVAL {
            return Vec::new();
        }
        self.last_poll = Instant::now();
        let mut changed = Vec::new();
        for (path, last) in &mut self.files {
            let now = modified(path);
            if now != *last {
                *last = now;
                changed.push(path.as_str());
            }
        }
        changed
    }
}
