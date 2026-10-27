//! シェーダまわりの共通部品

/// `f` の中で起きた wgpu の検証エラー（WGSL のコンパイルエラーなど）を、パニックではなく Err で返す。
/// ホットリロードで壊れたシェーダを読んでも落ちないようにするため
pub fn capture_validation<T>(device: &wgpu::Device, f: impl FnOnce() -> T) -> anyhow::Result<T> {
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let value = f();
    match pollster::block_on(scope.pop()) {
        None => Ok(value),
        Some(error) => Err(anyhow::anyhow!("{}", error)),
    }
}