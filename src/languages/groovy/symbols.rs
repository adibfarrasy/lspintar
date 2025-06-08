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

#[derive(Debug)]
pub struct SymbolInfo {
    pub name: String,
    pub symbol_type: SymbolType,
    pub scope_path: Vec<String>,
}
