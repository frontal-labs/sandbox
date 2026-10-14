//! Shell syntax tree.

/// How an item connects to the one after it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Connector {
    /// `;`, a newline, or end of input: always run the next item.
    Seq,
    /// `&&`: run the next item only if this one succeeded.
    And,
    /// `||`: run the next item only if this one failed.
    Or,
}

/// A list of items, such as a whole script or a loop body.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Script {
    pub items: Vec<Item>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub pipeline: Pipeline,
    pub connector: Connector,
}

/// Commands joined by `|`. Each stage reads the previous stage's stdout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pipeline {
    pub stages: Vec<Command>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Simple(SimpleCommand),
    If(IfClause),
    While(WhileClause),
    For(ForClause),
    /// `{ list; }`
    Group(Script),
    /// `( list )`, run in a copy of the shell state.
    Subshell(Script),
    /// `name() command`
    Function(FunctionDef),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IfClause {
    /// `if` and each `elif`: a condition and its body.
    pub branches: Vec<(Script, Script)>,
    pub otherwise: Option<Script>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhileClause {
    /// `until` loops run while the condition fails.
    pub until: bool,
    pub condition: Script,
    pub body: Script,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForClause {
    pub variable: String,
    pub words: Vec<Word>,
    pub body: Script,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionDef {
    pub name: String,
    pub body: Box<Command>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SimpleCommand {
    pub words: Vec<Word>,
    pub redirects: Vec<Redirect>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Redirect {
    pub kind: RedirectKind,
    pub target: Word,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RedirectKind {
    /// `< file`
    Input,
    /// `> file`
    Output,
    /// `>> file`
    Append,
}

/// How a character was quoted. This decides what expansion may touch it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Unquoted: variables expand and glob characters are active.
    Bare,
    /// Inside double quotes: variables expand, glob characters are literal.
    Double,
    /// Single quotes or backslash-escaped: nothing is special.
    Literal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ch {
    pub c: char,
    pub mode: Mode,
}

/// One part of a word: a character with its quoting, or a command substitution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Piece {
    Ch(Ch),
    /// `$(...)` or backticks. Its output replaces it, with trailing newlines removed.
    Subst(Script),
}

/// A shell word, kept in pieces until expansion.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Word(pub Vec<Piece>);

impl Word {
    /// The word's characters with quotes removed. Substitutions are skipped.
    pub fn text(&self) -> String {
        self.0
            .iter()
            .filter_map(|piece| match piece {
                Piece::Ch(ch) => Some(ch.c),
                Piece::Subst(_) => None,
            })
            .collect()
    }

    /// True if the word is one unquoted run of characters, with no substitution.
    /// Reserved words such as `if` and `done` must be like this.
    pub fn is_bare(&self) -> bool {
        self.0
            .iter()
            .all(|piece| matches!(piece, Piece::Ch(ch) if ch.mode == Mode::Bare))
    }
}
