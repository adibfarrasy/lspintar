pub mod gradle;

#[derive(Debug, Clone, PartialEq)]
pub enum BuildTool {
    Gradle,
    Maven,
}
