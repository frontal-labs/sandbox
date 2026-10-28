//! Tokenizer and recursive-descent parser for the syntax in `ast.rs`.

use crate::ast::{
    CaseArm, CaseClause, Ch, Command, Connector, ForClause, FunctionDef, IfClause, Item, Mode,
    Param, ParamOp, Piece, Pipeline, Redirect, RedirectKind, Script, SimpleCommand, WhileClause,
    Word,
};
use crate::expand::is_name;
use sandbox_types::{ErrorCode, SandboxError};
use std::iter::Peekable;
use std::str::Chars;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Word(Word),
    /// `>`, `>>`, `<`, or `>&`, with the descriptor written before it, if any.
    Redirect {
        fd: Option<u8>,
        kind: RedirectKind,
    },
    Pipe,
    And,
    Or,
    /// `;`
    Semi,
    /// A newline. It ends a command like `;`, but a list may not start with `;`.
    Newline,
    /// `;;`, which ends a `case` arm.
    DoubleSemi,
    LParen,
    RParen,
}

/// Deepest allowed `$(...)` / backtick nesting. Each level recurses into
/// `parse`, so an unbounded depth overflows the stack, which aborts the process
/// and cannot be caught. 32 levels is far beyond real scripts.
const MAX_NESTING: usize = 32;

/// Deepest allowed nesting of compound commands (`if`, `while`, `for`, `{ }`,
/// `( )`) in one script, and of calls, subshells, and substitutions at run time.
/// Each level recurses on the stack. A debug build overflows a 1 MiB stack at 64
/// levels and runs 32 levels in it, so 32 leaves a 2x margin on the 2 MiB stack
/// a tokio worker has (ADR 0018).
pub(crate) const MAX_COMMAND_DEPTH: usize = 32;

thread_local! {
    static NESTING: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Decrements the nesting depth when a `parse` call returns, including on error.
struct NestingGuard;

impl Drop for NestingGuard {
    fn drop(&mut self) {
        NESTING.with(|n| n.set(n.get() - 1));
    }
}

pub fn parse(input: &str) -> Result<Script, SandboxError> {
    let depth = NESTING.with(|n| {
        let depth = n.get() + 1;
        n.set(depth);
        depth
    });
    let _guard = NestingGuard;
    if depth > MAX_NESTING {
        return Err(SandboxError::new(
            ErrorCode::ResourceExhausted,
            format!("command substitution nested deeper than {MAX_NESTING} levels"),
        ));
    }
    let tokens = tokenize(input)?;
    let mut parser = Parser {
        tokens,
        pos: 0,
        depth: 0,
    };
    let script = parser.list(&[])?;
    if parser.pos != parser.tokens.len() {
        return Err(syntax("syntax error near unexpected token"));
    }
    Ok(script)
}

fn syntax(message: impl Into<String>) -> SandboxError {
    SandboxError::new(ErrorCode::ShellError, message)
}

/// A word being built. `started` is true once any character (even an empty
/// quoted one) has been seen, so `""` is a real word.
#[derive(Default)]
struct WordBuf {
    pieces: Vec<Piece>,
    started: bool,
    quoted: bool,
}

impl WordBuf {
    fn push(&mut self, c: char, mode: Mode) {
        self.started = true;
        if mode != Mode::Bare {
            self.quoted = true;
        }
        self.pieces.push(Piece::Ch(Ch { c, mode }));
    }

    fn push_piece(&mut self, piece: Piece) {
        self.started = true;
        self.pieces.push(piece);
    }

    fn start(&mut self) {
        self.started = true;
        self.quoted = true;
        self.pieces.push(Piece::Empty);
    }

    fn flush(&mut self, tokens: &mut Vec<Token>) {
        if self.started {
            tokens.push(Token::Word(Word(std::mem::take(&mut self.pieces))));
        }
        *self = WordBuf::default();
    }

    /// True if the pending word is a bare run of digits, as in `2>`.
    fn is_bare_fd(&self) -> bool {
        !self.quoted
            && !self.pieces.is_empty()
            && self.pieces.iter().all(
                |p| matches!(p, Piece::Ch(ch) if ch.mode == Mode::Bare && ch.c.is_ascii_digit()),
            )
    }
}

/// Splits `input` into tokens. In word mode, whitespace and operators are
/// ordinary characters, so the result is one word. This is how the operands
/// inside `${...}` are read.
fn tokenize_with(input: &str, word_mode: bool) -> Result<Vec<Token>, SandboxError> {
    let mut tokens = Vec::new();
    let mut word = WordBuf::default();
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                word.start();
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(ch) => word.push(ch, Mode::Literal),
                        None => return Err(syntax("unterminated single quote")),
                    }
                }
            }
            '"' => {
                word.start();
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') => match chars.peek().copied() {
                            Some('\n') => {
                                chars.next();
                            }
                            Some(next @ ('"' | '\\' | '$' | '`')) => {
                                chars.next();
                                word.push(next, Mode::Double);
                            }
                            _ => word.push('\\', Mode::Double),
                        },
                        Some('$') => match read_dollar(&mut chars, true)? {
                            Some(piece) => word.push_piece(piece),
                            None => word.push('$', Mode::Double),
                        },
                        Some('`') => word.push_piece(Piece::Subst {
                            script: read_backticks(&mut chars)?,
                            quoted: true,
                        }),
                        Some(ch) => word.push(ch, Mode::Double),
                        None => return Err(syntax("unterminated double quote")),
                    }
                }
            }
            '\\' => match chars.next() {
                // A backslash before a newline joins the two lines.
                Some('\n') => {}
                Some(ch) => {
                    word.start();
                    word.push(ch, Mode::Literal);
                }
                // A backslash at the very end is a literal backslash, as in bash.
                None => {
                    word.start();
                    word.push('\\', Mode::Literal);
                }
            },
            '$' => match read_dollar(&mut chars, false)? {
                Some(piece) => word.push_piece(piece),
                None => word.push('$', Mode::Bare),
            },
            '`' => word.push_piece(Piece::Subst {
                script: read_backticks(&mut chars)?,
                quoted: false,
            }),
            '#' if !word.started && !word_mode => {
                while let Some(&ch) = chars.peek() {
                    if ch == '\n' {
                        break;
                    }
                    chars.next();
                }
            }
            c if word_mode => word.push(c, Mode::Bare),
            '\n' => {
                word.flush(&mut tokens);
                tokens.push(Token::Newline);
            }
            c if c.is_whitespace() => word.flush(&mut tokens),
            ';' => {
                word.flush(&mut tokens);
                if chars.peek() == Some(&';') {
                    chars.next();
                    tokens.push(Token::DoubleSemi);
                } else {
                    tokens.push(Token::Semi);
                }
            }
            '(' => {
                word.flush(&mut tokens);
                tokens.push(Token::LParen);
            }
            ')' => {
                word.flush(&mut tokens);
                tokens.push(Token::RParen);
            }
            '|' => {
                word.flush(&mut tokens);
                if chars.peek() == Some(&'|') {
                    chars.next();
                    tokens.push(Token::Or);
                } else {
                    tokens.push(Token::Pipe);
                }
            }
            '&' => {
                word.flush(&mut tokens);
                if chars.peek() == Some(&'&') {
                    chars.next();
                    tokens.push(Token::And);
                } else {
                    return Err(syntax("background jobs (&) are not supported"));
                }
            }
            '>' | '<' => {
                let fd = if word.is_bare_fd() {
                    let digits: String = word.pieces.iter().filter_map(piece_char).collect();
                    word = WordBuf::default();
                    let n: u8 = digits
                        .parse()
                        .map_err(|_| syntax(format!("file descriptor {digits} is too large")))?;
                    Some(n)
                } else {
                    word.flush(&mut tokens);
                    None
                };
                let kind = if c == '<' {
                    RedirectKind::Input
                } else if chars.peek() == Some(&'>') {
                    chars.next();
                    RedirectKind::Append
                } else if chars.peek() == Some(&'&') {
                    chars.next();
                    RedirectKind::Dup
                } else {
                    RedirectKind::Output
                };
                tokens.push(Token::Redirect { fd, kind });
            }
            other => word.push(other, Mode::Bare),
        }
    }
    word.flush(&mut tokens);
    Ok(tokens)
}

fn tokenize(input: &str) -> Result<Vec<Token>, SandboxError> {
    tokenize_with(input, false)
}

/// The character of a bare piece, used to read a file descriptor number.
fn piece_char(piece: &Piece) -> Option<char> {
    match piece {
        Piece::Ch(ch) => Some(ch.c),
        _ => None,
    }
}

/// Reads what follows a `$`: a parameter reference, a substitution, or arithmetic.
/// Returns `None` when the `$` stands alone, so the caller keeps it as a literal.
fn read_dollar(
    chars: &mut Peekable<Chars<'_>>,
    quoted: bool,
) -> Result<Option<Piece>, SandboxError> {
    match chars.peek().copied() {
        // `$name`, `$N` (one digit, as in POSIX), and the special parameters.
        Some(c) if c == '_' || c.is_ascii_alphabetic() => {
            let mut name = String::new();
            while let Some(&c) = chars.peek() {
                if c == '_' || c.is_ascii_alphanumeric() {
                    name.push(c);
                    chars.next();
                } else {
                    break;
                }
            }
            Ok(Some(plain_param(name, quoted)))
        }
        Some(c) if c.is_ascii_digit() || matches!(c, '?' | '#' | '@' | '*') => {
            chars.next();
            Ok(Some(plain_param(c.to_string(), quoted)))
        }
        Some('(') => {
            chars.next();
            if chars.peek() == Some(&'(') {
                chars.next();
                let text = read_arithmetic(chars)?;
                return Ok(Some(Piece::Arith(word_from_text(&text)?)));
            }
            Ok(Some(Piece::Subst {
                script: read_substitution(chars)?,
                quoted,
            }))
        }
        Some('{') => {
            chars.next();
            let raw = read_braced(chars)?;
            Ok(Some(Piece::Param(parse_param(&raw, quoted)?)))
        }
        _ => Ok(None),
    }
}

fn plain_param(name: String, quoted: bool) -> Piece {
    Piece::Param(Param {
        name,
        op: ParamOp::Value,
        quoted,
    })
}

/// Reads the text of `$(( ... ))` up to its closing `))`. The text is then
/// parsed as a word, so its `$` expansions and substitutions run first.
fn read_arithmetic(chars: &mut Peekable<Chars<'_>>) -> Result<String, SandboxError> {
    let mut text = String::new();
    let mut depth = 0usize;
    loop {
        match chars.next() {
            Some('(') => {
                depth += 1;
                text.push('(');
            }
            Some(')') if depth == 0 => {
                if chars.peek() == Some(&')') {
                    chars.next();
                    return Ok(text);
                }
                return Err(syntax("arithmetic expression needs '))'"));
            }
            Some(')') => {
                depth -= 1;
                text.push(')');
            }
            Some(c) => text.push(c),
            None => return Err(syntax("unterminated $(( ))")),
        }
    }
}

/// Reads the text of `${ ... }` up to its closing brace. Quotes and nested
/// braces are respected, so `${x:-"}"}` is one expansion.
fn read_braced(chars: &mut Peekable<Chars<'_>>) -> Result<String, SandboxError> {
    let mut text = String::new();
    let mut depth = 0usize;
    let mut quote: Option<char> = None;
    loop {
        let Some(c) = chars.next() else {
            return Err(syntax("unterminated ${"));
        };
        match (quote, c) {
            (None, '}') if depth == 0 => return Ok(text),
            (None, '}') => depth -= 1,
            (None, '{') => depth += 1,
            (None, '\'' | '"') => quote = Some(c),
            (Some(q), c) if c == q => quote = None,
            _ => {}
        }
        text.push(c);
    }
}

/// Parses the inside of `${ ... }`: a name and an optional operator.
fn parse_param(raw: &str, quoted: bool) -> Result<Param, SandboxError> {
    let bad = || syntax(format!("bad substitution: ${{{raw}}}"));
    // `${!name}` and `${!name:-word}` take the value of the variable `name` names.
    if let Some(rest) = raw.strip_prefix('!') {
        let (name, rest) = parse_name(rest).ok_or_else(bad)?;
        let default = if rest.is_empty() {
            None
        } else if let Some(word) = rest.strip_prefix(":-") {
            Some((true, word_from_text(word)?))
        } else if let Some(word) = rest.strip_prefix('-') {
            Some((false, word_from_text(word)?))
        } else {
            return Err(bad());
        };
        return Ok(Param {
            name: name.to_string(),
            op: ParamOp::Indirect { default },
            quoted,
        });
    }
    // `${#name}` is the length. `${#}` alone is the count of positional parameters.
    if let Some(rest) = raw.strip_prefix('#') {
        if !rest.is_empty() && !rest.starts_with(['%', '#', ':', '-']) {
            return Ok(Param {
                name: parse_name(rest).ok_or_else(bad)?.0.to_string(),
                op: ParamOp::Length,
                quoted,
            });
        }
    }
    let (name, rest) = parse_name(raw).ok_or_else(bad)?;
    let op = if rest.is_empty() {
        ParamOp::Value
    } else if let Some(word) = rest.strip_prefix(":-") {
        ParamOp::Default {
            colon: true,
            word: word_from_text(word)?,
        }
    } else if let Some(word) = rest.strip_prefix('-') {
        ParamOp::Default {
            colon: false,
            word: word_from_text(word)?,
        }
    } else if let Some(pattern) = rest.strip_prefix("%%") {
        ParamOp::Suffix {
            longest: true,
            pattern: word_from_text(pattern)?,
        }
    } else if let Some(pattern) = rest.strip_prefix('%') {
        ParamOp::Suffix {
            longest: false,
            pattern: word_from_text(pattern)?,
        }
    } else if let Some(pattern) = rest.strip_prefix("##") {
        ParamOp::Prefix {
            longest: true,
            pattern: word_from_text(pattern)?,
        }
    } else if let Some(pattern) = rest.strip_prefix('#') {
        ParamOp::Prefix {
            longest: false,
            pattern: word_from_text(pattern)?,
        }
    } else {
        return Err(bad());
    };
    Ok(Param {
        name: name.to_string(),
        op,
        quoted,
    })
}

/// Splits a parameter name off the front of `text`. A name is a variable name,
/// a single special character, or a run of digits.
fn parse_name(text: &str) -> Option<(&str, &str)> {
    let mut chars = text.char_indices();
    let (_, first) = chars.next()?;
    if first == '_' || first.is_ascii_alphabetic() {
        let end = text
            .char_indices()
            .find(|(_, c)| !(*c == '_' || c.is_ascii_alphanumeric()))
            .map_or(text.len(), |(i, _)| i);
        return Some((&text[..end], &text[end..]));
    }
    if first.is_ascii_digit() {
        let end = text
            .char_indices()
            .find(|(_, c)| !c.is_ascii_digit())
            .map_or(text.len(), |(i, _)| i);
        return Some((&text[..end], &text[end..]));
    }
    if matches!(first, '?' | '#' | '@' | '*' | '$') {
        return Some((&text[..first.len_utf8()], &text[first.len_utf8()..]));
    }
    None
}

/// Reads an operand of a parameter expansion as a word. Spaces and operators
/// are literal there, and quotes and `$` work as they do elsewhere.
fn word_from_text(text: &str) -> Result<Word, SandboxError> {
    let mut tokens = tokenize_with(text, true)?;
    match tokens.pop() {
        None => Ok(Word::default()),
        Some(Token::Word(word)) if tokens.is_empty() => Ok(word),
        _ => Err(syntax(format!("bad substitution operand: {text}"))),
    }
}

/// Reads the body of `$( ... )` up to its matching `)` and parses it.
fn read_substitution(chars: &mut Peekable<Chars<'_>>) -> Result<Script, SandboxError> {
    let mut inner = String::new();
    let mut depth = 1usize;
    let mut quote: Option<char> = None;
    loop {
        let Some(c) = chars.next() else {
            return Err(syntax("unterminated $("));
        };
        match quote {
            Some('\'') => {
                if c == '\'' {
                    quote = None;
                }
            }
            Some('"') => match c {
                '"' => quote = None,
                '\\' => {
                    inner.push(c);
                    if let Some(next) = chars.next() {
                        inner.push(next);
                    }
                    continue;
                }
                _ => {}
            },
            _ => match c {
                '\\' => {
                    inner.push(c);
                    if let Some(next) = chars.next() {
                        inner.push(next);
                    }
                    continue;
                }
                '\'' | '"' => quote = Some(c),
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            },
        }
        inner.push(c);
    }
    parse(&inner)
}

/// Reads a backtick substitution up to the closing backtick and parses it.
fn read_backticks(chars: &mut Peekable<Chars<'_>>) -> Result<Script, SandboxError> {
    let mut inner = String::new();
    loop {
        match chars.next() {
            Some('`') => break,
            Some('\\') => {
                if let Some(next) = chars.next() {
                    inner.push(next);
                }
            }
            Some(c) => inner.push(c),
            None => return Err(syntax("unterminated backtick substitution")),
        }
    }
    parse(&inner)
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    /// Compound commands open around the current position.
    depth: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn bump(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.pos).cloned();
        self.pos += 1;
        token
    }

    /// The reserved word at the cursor, if the cursor is on a bare word.
    fn keyword(&self) -> Option<String> {
        match self.peek() {
            Some(Token::Word(word)) if word.is_bare() => Some(word.text()),
            _ => None,
        }
    }

    fn expect_keyword(&mut self, keyword: &str) -> Result<(), SandboxError> {
        if self.keyword().as_deref() == Some(keyword) {
            self.pos += 1;
            Ok(())
        } else {
            Err(syntax(format!("syntax error: expected '{keyword}'")))
        }
    }

    fn expect(&mut self, token: &Token, what: &str) -> Result<(), SandboxError> {
        if self.peek() == Some(token) {
            self.pos += 1;
            Ok(())
        } else {
            Err(syntax(format!("syntax error: expected {what}")))
        }
    }

    /// Skips newlines and `;` between commands.
    fn skip_separators(&mut self) {
        while matches!(self.peek(), Some(Token::Semi | Token::Newline)) {
            self.pos += 1;
        }
    }

    /// A list of pipelines. Stops before a reserved word in `stops`, before
    /// `)` or `;;`, or at the end of input.
    fn list(&mut self, stops: &[&str]) -> Result<Script, SandboxError> {
        let mut items = Vec::new();
        loop {
            // A list may begin with newlines, but not with `;`.
            while matches!(self.peek(), Some(Token::Newline)) {
                self.pos += 1;
            }
            if items.is_empty() && matches!(self.peek(), Some(Token::Semi)) {
                return Err(syntax("syntax error near unexpected token ';'"));
            }
            self.skip_separators();
            match self.peek() {
                None | Some(Token::RParen | Token::DoubleSemi) => break,
                _ => {}
            }
            if let Some(keyword) = self.keyword() {
                if stops.contains(&keyword.as_str()) {
                    break;
                }
            }

            let pipeline = self.pipeline()?;
            let connector = match self.peek() {
                Some(Token::And) => {
                    self.pos += 1;
                    Connector::And
                }
                Some(Token::Or) => {
                    self.pos += 1;
                    Connector::Or
                }
                _ => Connector::Seq,
            };
            // `&&` and `||` need a command after them. Newlines may come first.
            if connector != Connector::Seq {
                while matches!(self.peek(), Some(Token::Newline)) {
                    self.pos += 1;
                }
                if matches!(self.peek(), None | Some(Token::RParen)) {
                    return Err(syntax("syntax error: command expected after operator"));
                }
            }
            // After a complete pipeline only a separator, an operator, or the
            // end of a list may follow.
            match self.peek() {
                None | Some(Token::Semi | Token::Newline | Token::RParen | Token::DoubleSemi) => {}
                Some(_) if connector != Connector::Seq => {}
                Some(_) if self.keyword().is_some_and(|k| stops.contains(&k.as_str())) => {}
                Some(_) => return Err(syntax("syntax error near unexpected token")),
            }
            items.push(Item {
                pipeline,
                connector,
            });
        }
        Ok(Script { items })
    }

    fn pipeline(&mut self) -> Result<Pipeline, SandboxError> {
        let negated = self.keyword().as_deref() == Some("!");
        if negated {
            self.pos += 1;
        }
        let mut stages = vec![self.command()?];
        while matches!(self.peek(), Some(Token::Pipe)) {
            self.pos += 1;
            stages.push(self.command()?);
        }
        Ok(Pipeline { stages, negated })
    }

    fn command(&mut self) -> Result<Command, SandboxError> {
        if self.depth >= MAX_COMMAND_DEPTH {
            return Err(SandboxError::new(
                ErrorCode::ResourceExhausted,
                format!("commands nested deeper than {MAX_COMMAND_DEPTH} levels"),
            ));
        }
        self.depth += 1;
        let parsed = self.command_at_depth();
        self.depth -= 1;
        parsed
    }

    fn command_at_depth(&mut self) -> Result<Command, SandboxError> {
        match self.peek() {
            None
            | Some(
                Token::Semi
                | Token::Newline
                | Token::And
                | Token::Or
                | Token::Pipe
                | Token::RParen
                | Token::DoubleSemi,
            ) => Err(syntax("syntax error: command expected")),
            Some(Token::LParen) => {
                self.pos += 1;
                let body = self.list(&[])?;
                self.expect(&Token::RParen, "')'")?;
                self.with_redirects(Command::Subshell(body))
            }
            Some(Token::Word(_)) => match self.keyword().as_deref() {
                Some("if") => {
                    let command = self.if_clause()?;
                    self.with_redirects(command)
                }
                Some("while") => {
                    let command = self.while_clause("while")?;
                    self.with_redirects(command)
                }
                Some("until") => {
                    let command = self.while_clause("until")?;
                    self.with_redirects(command)
                }
                Some("for") => {
                    let command = self.for_clause()?;
                    self.with_redirects(command)
                }
                Some("case") => {
                    let command = self.case_clause()?;
                    self.with_redirects(command)
                }
                Some("{") => {
                    self.pos += 1;
                    let body = self.list(&["}"])?;
                    self.expect_keyword("}")?;
                    self.with_redirects(Command::Group(body))
                }
                Some(
                    keyword @ ("then" | "elif" | "else" | "fi" | "do" | "done" | "}" | "esac"),
                ) => Err(syntax(format!(
                    "syntax error near unexpected token '{keyword}'"
                ))),
                _ if self.is_function_start() => self.function(),
                _ => self.simple(),
            },
            Some(_) => self.simple(),
        }
    }

    /// Wraps a compound command with the redirections written after it.
    fn with_redirects(&mut self, command: Command) -> Result<Command, SandboxError> {
        let redirects = self.redirects()?;
        if redirects.is_empty() {
            Ok(command)
        } else {
            Ok(Command::Redirected {
                command: Box::new(command),
                redirects,
            })
        }
    }

    /// Reads the redirections at the cursor. Each one is an operator and a word.
    fn redirects(&mut self) -> Result<Vec<Redirect>, SandboxError> {
        let mut redirects = Vec::new();
        while let Some(Token::Redirect { fd, kind }) = self.peek().cloned() {
            self.pos += 1;
            let target = match self.bump() {
                Some(Token::Word(target)) => target,
                _ => return Err(syntax("redirection needs a file name")),
            };
            let default_fd = if kind == RedirectKind::Input { 0 } else { 1 };
            let fd = fd.unwrap_or(default_fd);
            if fd > 2 {
                return Err(syntax(format!(
                    "file descriptor {fd} is not supported; only 0, 1, and 2 are"
                )));
            }
            if kind == RedirectKind::Input && fd != 0 {
                return Err(syntax("only descriptor 0 can be redirected from"));
            }
            if kind != RedirectKind::Input && fd == 0 {
                return Err(syntax("descriptor 0 can only be read from"));
            }
            redirects.push(Redirect { fd, kind, target });
        }
        Ok(redirects)
    }

    /// `name ( )` starts a function definition.
    fn is_function_start(&self) -> bool {
        let is_name_word =
            matches!(self.peek(), Some(Token::Word(w)) if w.is_bare() && is_name(&w.text()));
        is_name_word
            && matches!(self.tokens.get(self.pos + 1), Some(Token::LParen))
            && matches!(self.tokens.get(self.pos + 2), Some(Token::RParen))
    }

    fn function(&mut self) -> Result<Command, SandboxError> {
        let name = match self.bump() {
            Some(Token::Word(word)) => word.text(),
            _ => return Err(syntax("syntax error: function name expected")),
        };
        self.pos += 2; // `(` and `)`
        self.skip_separators();
        let body = self.command()?;
        Ok(Command::Function(FunctionDef {
            name,
            body: Box::new(body),
        }))
    }

    fn if_clause(&mut self) -> Result<Command, SandboxError> {
        self.pos += 1; // if
        let mut branches = Vec::new();
        let condition = self.list(&["then"])?;
        self.expect_keyword("then")?;
        let body = self.list(&["elif", "else", "fi"])?;
        branches.push((condition, body));

        let mut otherwise = None;
        loop {
            match self.keyword().as_deref() {
                Some("elif") => {
                    self.pos += 1;
                    let condition = self.list(&["then"])?;
                    self.expect_keyword("then")?;
                    let body = self.list(&["elif", "else", "fi"])?;
                    branches.push((condition, body));
                }
                Some("else") => {
                    self.pos += 1;
                    otherwise = Some(self.list(&["fi"])?);
                    self.expect_keyword("fi")?;
                    break;
                }
                Some("fi") => {
                    self.pos += 1;
                    break;
                }
                _ => return Err(syntax("syntax error: expected 'fi'")),
            }
        }
        Ok(Command::If(IfClause {
            branches,
            otherwise,
        }))
    }

    fn while_clause(&mut self, keyword: &str) -> Result<Command, SandboxError> {
        self.pos += 1;
        let condition = self.list(&["do"])?;
        self.expect_keyword("do")?;
        let body = self.list(&["done"])?;
        self.expect_keyword("done")?;
        Ok(Command::While(WhileClause {
            until: keyword == "until",
            condition,
            body,
        }))
    }

    fn for_clause(&mut self) -> Result<Command, SandboxError> {
        self.pos += 1;
        let variable = match self.bump() {
            Some(Token::Word(word)) if word.is_bare() && is_name(&word.text()) => word.text(),
            _ => return Err(syntax("syntax error: for needs a variable name")),
        };
        if self.keyword().as_deref() != Some("in") {
            return Err(syntax(
                "syntax error: for needs 'in' (the implicit list is not supported)",
            ));
        }
        self.pos += 1;
        let mut words = Vec::new();
        while let Some(Token::Word(word)) = self.peek() {
            words.push(word.clone());
            self.pos += 1;
        }
        self.skip_separators();
        self.expect_keyword("do")?;
        let body = self.list(&["done"])?;
        self.expect_keyword("done")?;
        Ok(Command::For(ForClause {
            variable,
            words,
            body,
        }))
    }

    /// `case WORD in pattern [| pattern]...) list ;; ... esac`. The last arm may
    /// leave out its `;;`.
    fn case_clause(&mut self) -> Result<Command, SandboxError> {
        self.pos += 1; // case
        let subject = match self.bump() {
            Some(Token::Word(word)) => word,
            _ => return Err(syntax("syntax error: case needs a word")),
        };
        self.skip_separators();
        self.expect_keyword("in")?;
        let mut arms = Vec::new();
        loop {
            self.skip_separators();
            if self.keyword().as_deref() == Some("esac") {
                self.pos += 1;
                break;
            }
            if self.peek().is_none() {
                return Err(syntax("syntax error: expected 'esac'"));
            }
            if matches!(self.peek(), Some(Token::LParen)) {
                self.pos += 1;
            }
            let mut patterns = Vec::new();
            loop {
                match self.bump() {
                    Some(Token::Word(pattern)) => patterns.push(pattern),
                    _ => return Err(syntax("syntax error: case pattern expected")),
                }
                match self.bump() {
                    Some(Token::Pipe) => continue,
                    Some(Token::RParen) => break,
                    _ => return Err(syntax("syntax error: expected ')' after case pattern")),
                }
            }
            let body = self.list(&["esac"])?;
            arms.push(CaseArm { patterns, body });
            match self.peek() {
                Some(Token::DoubleSemi) => self.pos += 1,
                _ if self.keyword().as_deref() == Some("esac") => {}
                _ => return Err(syntax("syntax error: expected ';;' or 'esac'")),
            }
        }
        Ok(Command::Case(CaseClause { subject, arms }))
    }

    fn simple(&mut self) -> Result<Command, SandboxError> {
        let mut command = SimpleCommand::default();
        loop {
            match self.peek() {
                Some(Token::Word(word)) => {
                    command.words.push(word.clone());
                    self.pos += 1;
                }
                Some(Token::Redirect { .. }) => {
                    command.redirects.extend(self.redirects()?);
                }
                _ => break,
            }
        }
        if command.words.is_empty() && command.redirects.is_empty() {
            return Err(syntax("syntax error: missing command"));
        }
        Ok(Command::Simple(command))
    }
}
