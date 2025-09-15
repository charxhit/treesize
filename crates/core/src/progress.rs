#[derive(Debug, Clone, Default)]
pub struct Progress {
    pub files: u64,
    pub dirs: u64,
    pub bytes: u128,
}
