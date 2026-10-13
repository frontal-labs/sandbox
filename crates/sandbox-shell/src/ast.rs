//! Shell AST definitions

#[derive(Debug, Clone, PartialEq)]
pub enum AstNode {
    Command(Command),
    Pipeline(Vec<Command>),
    Sequence(Vec<Command>),
    Background(Box<Command>),
    Subshell(Box<Command>),
    Redirect(Box<Command>, Redirect),
    If(IfStatement),
    While(WhileStatement),
    For(ForStatement),
    Case(CaseStatement),
    FunctionDef(FunctionDef),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Command {
    pub name: String,
    pub args: Vec<String>,
    pub redirects: Vec<Redirect>,
    pub background: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Redirect {
    Stdout(String),
    Stderr(String),
    StdoutAppend(String),
    StderrAppend(String),
    Stdin(String),
    HereDoc(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct IfStatement {
    pub condition: Box<Command>,
    pub then_branch: Vec<AstNode>,
    pub else_branch: Option<Vec<AstNode>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WhileStatement {
    pub condition: Box<Command>,
    pub body: Vec<AstNode>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ForStatement {
    pub variable: String,
    pub items: Vec<String>,
    pub body: Vec<AstNode>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CaseStatement {
    pub expression: String,
    pub cases: Vec<(String, Vec<AstNode>)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FunctionDef {
    pub name: String,
    pub body: Vec<AstNode>,
}