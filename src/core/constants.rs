pub const SOURCE_DIRS: [&str; 4] = [
    "src/main/java",
    "src/test/java",
    "src/main/groovy",
    "src/test/groovy",
];

pub const EXTENSIONS: [&str; 5] = ["java", "kt", "gradle", "kts", "groovy"];

pub const PROJECT_ROOT_MARKER: [&str; 4] = ["build.gradle", "build.gradle.kts", "pom.xml", ".git"];

// https://groovy-lang.org/differences.html
pub const GROOVY_DEFAULT_IMPORTS: &[&str] = &[
    "java.io.*",
    "java.lang.*",
    "java.math.BigDecimal",
    "java.math.BigInteger",
    "java.net.*",
    "java.util.*",
    "groovy.lang.*",
    "groovy.util.*",
];
