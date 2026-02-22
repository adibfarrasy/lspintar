use std::{env, path::PathBuf, sync::OnceLock};

pub static CACHE_DIR: OnceLock<PathBuf> = OnceLock::new();
pub static CFR_JAR_PATH: OnceLock<Option<PathBuf>> = OnceLock::new();
pub const MAX_LINE_COUNT: usize = 10_000;
pub const FILE_CACHE_TTL_SECS: u64 = 30;

pub fn get_cache_dir() -> &'static PathBuf {
    CACHE_DIR.get_or_init(|| {
        dirs::cache_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join("lspintar/caches")
    })
}

pub fn get_cfr_jar_path() -> &'static Option<PathBuf> {
    CFR_JAR_PATH.get_or_init(|| env::var("CFR_JAR_PATH").ok().map(PathBuf::from))
}
