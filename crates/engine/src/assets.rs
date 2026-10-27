//! ファイル読み込みの窓口。
//! 読み込みはすべてここを通す。将来は同梱素材やネットワーク経由（スマホでのホットリロード）に差し替える。

use std::time::SystemTime;

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
