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

/// Commands joined by `|`. Each stage reads the previous stage's stdout. A
/// leading `!` negates the status of the whole pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pipeline {
    pub stages: Vec<Command>,
    pub negated: bool,
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
    /// `case WORD in ... esac`
    Case(CaseClause),
    /// A compound command with redirections after it, such as `for ...; done > f`.
    Redirected {
        command: Box<Command>,
        redirects: Vec<Redirect>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaseClause {
    pub subject: Word,
    pub arms: Vec<CaseArm>,
}

/// `pattern | pattern ) body ;;`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaseArm {
    pub patterns: Vec<Word>,
    pub body: Script,
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
    /// The file descriptor redirected: 0 for input, 1 or 2 for output. Defaults
    /// to 0 for `<` and 1 for `>` and `>>`.
    pub fd: u8,
    pub kind: RedirectKind,
    /// The file name, or for `Dup`, the descriptor number written as a word.
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
    /// `>&N`: this descriptor becomes a copy of descriptor N.
    Dup,
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
    /// Output of an unquoted expansion. It is split into fields on `IFS`, and
    /// it is not globbed.
    Expanded,
    /// A field break from `"$@"`: one per positional parameter.
    Break,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ch {
    pub c: char,
    pub mode: Mode,
}

/// One part of a word: a character with its quoting, or an expansion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Piece {
    Ch(Ch),
    /// `$(...)` or backticks. Its output replaces it, with trailing newlines
    /// removed. `quoted` is true inside double quotes, where the output is not
    /// split into fields.
    Subst {
        script: Script,
        quoted: bool,
    },
    /// `${name}` and the forms with an operator, such as `${name:-word}`.
    Param(Param),
    /// `$((expr))`. The expression is expanded like a word, then evaluated as
    /// integer arithmetic.
    Arith(Word),
    /// The mark a quoted word leaves when it has no characters, as `''` does.
    /// It makes the word one empty field.
    Empty,
}

/// A parameter expansion: `${name}` with an optional operator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Param {
    pub name: String,
    pub op: ParamOp,
    /// True inside double quotes. Quoted results are not split into fields.
    pub quoted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParamOp {
    /// `$name` or `${name}`
    Value,
    /// `${#name}`: the length of the value
    Length,
    /// `${name:-word}` (`colon`) or `${name-word}`: the word when unset, or
    /// also when empty if `colon` is set.
    Default { colon: bool, word: Word },
    /// `${name%pattern}` (`longest` false) or `${name%%pattern}`: remove a suffix.
    Suffix { longest: bool, pattern: Word },
    /// `${name#pattern}` or `${name##pattern}`: remove a prefix.
    Prefix { longest: bool, pattern: Word },
    /// `${!name}`: the value of the variable whose name is the value of `name`.
    /// With `default`, the given word is used when that value is unset (or
    /// empty, if the colon form is used).
    Indirect { default: Option<(bool, Word)> },
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
                Piece::Subst { .. } | Piece::Param(_) | Piece::Arith(_) | Piece::Empty => None,
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
