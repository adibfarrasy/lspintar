use std::{path::PathBuf, sync::OnceLock};

pub static CACHE_DIR: OnceLock<PathBuf> = OnceLock::new();
const CFR_JAR: &[u8] = include_bytes!("../../vendor/cfr.jar");
pub const MAX_LINE_COUNT: usize = 10_000;
pub const FILE_CACHE_TTL_SECS: u64 = 30;

pub fn get_cache_dir() -> &'static PathBuf {
    CACHE_DIR.get_or_init(|| {
        dirs::cache_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join("lspintar/caches")
    })
}

pub fn get_cfr_jar_path() -> PathBuf {
    let path = get_cache_dir().join("cfr.jar");
    if !path.exists() {
        std::fs::write(&path, CFR_JAR).expect("failed to extract cfr.jar");
    }
    path
}

pub const MANIFEST_PATH_FRAGMENT: &str = ".lspintar/deps.manifest";
pub const INDEX_PATH_FRAGMENT: &str = ".lspintar/index.version";
pub const DB_PATH_FRAGMENT: &str = ".lspintar/index.db";

pub const INDEX_VERSION: &str = env!("CARGO_PKG_VERSION");
