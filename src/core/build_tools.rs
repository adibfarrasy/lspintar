use std::path::Path;

#[derive(Debug, Clone)]
pub enum BuildTool {
    Gradle,
    Maven,
}

pub fn detect_build_tool(project_root: &Path) -> Option<BuildTool> {
    if project_root.join("build.gradle").exists() || project_root.join("build.gradle.kts").exists()
    {
        return Some(BuildTool::Gradle);
    }

    if project_root.join("pom.xml").exists() {
        return Some(BuildTool::Maven);
    }

    None
}
