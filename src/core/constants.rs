use std::sync::OnceLock;

pub const SOURCE_DIRS: [&str; 6] = [
    "src/main/java",
    "src/test/java",
    "src/main/groovy",
    "src/test/groovy",
    "src/main/kotlin",
    "src/test/kotlin",
];

pub const EXTENSIONS: [&str; 5] = ["java", "kt", "gradle", "kts", "groovy"];

pub const PROJECT_ROOT_MARKER: [&str; 6] = ["build.gradle", "build.gradle.kts", "settings.gradle", "settings.gradle.kts", "pom.xml", ".git"];

pub static GROOVY_PARSER: OnceLock<tree_sitter::Language> = OnceLock::new();
pub static JAVA_PARSER: OnceLock<tree_sitter::Language> = OnceLock::new();
pub static KOTLIN_PARSER: OnceLock<tree_sitter::Language> = OnceLock::new();

pub const IS_INDEXING_COMPLETED: &str = "is_indexing_completed";
pub const GRADLE_CACHE_DIR: &str = "gradle_cache_dir";
pub const BUILD_ON_INIT: &str = "build_on_init";

pub const TEMP_DIR_PREFIX: &str = "lspintar_builtin_sources";
