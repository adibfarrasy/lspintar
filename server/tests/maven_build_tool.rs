use lsp_core::build_tools::{BuildToolHandler, maven::MavenHandler};
use std::path::Path;

#[test]
fn detects_maven_project_by_pom_xml() {
    let handler = MavenHandler;
    assert!(handler.is_project(Path::new("tests/fixtures/java-maven-single")));
    assert!(handler.is_project(Path::new("tests/fixtures/java-maven-multi")));
    assert!(!handler.is_project(Path::new("tests/fixtures/groovy-gradle-single")));
}

#[test]
fn is_build_file_matches_pom_xml_only() {
    let handler = MavenHandler;
    assert!(handler.is_build_file(Path::new("pom.xml")));
    assert!(!handler.is_build_file(Path::new("build.gradle")));
}

#[test]
fn resolves_dependency_jar_paired_with_its_source_jar() {
    let handler = MavenHandler;
    let path = Path::new("tests/fixtures/java-maven-single");

    let pairs = handler
        .get_dependency_paths(path)
        .expect("dependency resolution failed");

    let has_paired_groovy_json = pairs.iter().any(|(jar, src)| {
        let jar_matches = jar
            .as_ref()
            .is_some_and(|p| p.to_string_lossy().contains("groovy-json") && !p.to_string_lossy().contains("-sources"));
        let src_matches = src
            .as_ref()
            .is_some_and(|p| p.to_string_lossy().contains("groovy-json-4.0.15-sources.jar"));
        jar_matches && src_matches
    });

    assert!(
        has_paired_groovy_json,
        "expected a (jar, source) pair for groovy-json, got: {pairs:?}"
    );
}

#[test]
fn resolves_jdk_source_zip() {
    let handler = MavenHandler;
    let path = Path::new("tests/fixtures/java-maven-single");

    let jdk_src = handler
        .get_jdk_dependency_path(path)
        .expect("JDK dependency resolution failed");

    assert!(jdk_src.is_some(), "expected a JDK src.zip to be found");
    assert!(jdk_src.unwrap().to_string_lossy().ends_with("src.zip"));
}

#[test]
fn resolves_multi_module_subproject_classpath() {
    let handler = MavenHandler;
    let path = Path::new("tests/fixtures/java-maven-multi");

    let subprojects = handler
        .get_subproject_classpath(path)
        .expect("subproject classpath resolution failed");

    assert_eq!(subprojects.len(), 2, "expected core + app modules");

    let core = subprojects
        .iter()
        .find(|s| s.source_dirs.iter().any(|d| d.ends_with("core/src/main/java")))
        .expect("core module not found");
    assert!(
        core.jar_paths.iter().any(|p| p.to_string_lossy().contains("groovy-json")),
        "core classpath should include groovy-json, got: {:?}",
        core.jar_paths
    );

    let app = subprojects
        .iter()
        .find(|s| s.source_dirs.iter().any(|d| d.ends_with("app/src/main/java")))
        .expect("app module not found");
    assert!(
        app.jar_paths.iter().any(|p| p.to_string_lossy().contains("core")),
        "app classpath should include the core reactor module's jar, got: {:?}",
        app.jar_paths
    );
}
