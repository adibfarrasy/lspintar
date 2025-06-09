#[derive(Debug, Clone, PartialEq)]
pub enum SymbolType {
    Variable,
    Function,
    Method,
    Type,
    Class,
    Interface,
    Annotation,
    Enum,
    Field,
    Property,
    Module,
    Package,
    Constant,
    Parameter,
    LocalVariable,
}
