//! Tokenizer and recursive-descent parser for the syntax in `ast.rs`.

use crate::ast::{
    Ch, Command, Connector, ForClause, FunctionDef, IfClause, Item, Mode, Piece, Pipeline,
    Redirect, RedirectKind, Script, SimpleCommand, WhileClause, Word,
};
use crate::expand::is_name;
use sandbox_types::{ErrorCode, SandboxError};
use std::iter::Peekable;
use std::str::Chars;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Word(Word),
    Pipe,
    And,
    Or,
    Semi,
    LParen,
    RParen,
    Input,
    Output,
    Append,
}

pub fn parse(input: &str) -> Result<Script, SandboxError> {
    let tokens = tokenize(input)?;
    let mut parser = Parser { tokens, pos: 0 };
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

    fn push_subst(&mut self, script: Script) {
        self.started = true;
        self.pieces.push(Piece::Subst(script));
    }

    fn start(&mut self) {
        self.started = true;
        self.quoted = true;
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

fn tokenize(input: &str) -> Result<Vec<Token>, SandboxError> {
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
                            Some(next @ ('"' | '\\' | '$' | '`')) => {
                                chars.next();
                                word.push(next, Mode::Double);
                            }
                            _ => word.push('\\', Mode::Double),
                        },
                        Some('$') if chars.peek() == Some(&'(') => {
                            chars.next();
                            word.push_subst(read_substitution(&mut chars)?);
                        }
                        Some('`') => word.push_subst(read_backticks(&mut chars)?),
                        Some(ch) => word.push(ch, Mode::Double),
                        None => return Err(syntax("unterminated double quote")),
                    }
                }
            }
            '\\' => match chars.next() {
                Some(ch) => {
                    word.start();
                    word.push(ch, Mode::Literal);
                }
                None => return Err(syntax("trailing backslash")),
            },
            '$' if chars.peek() == Some(&'(') => {
                chars.next();
                word.push_subst(read_substitution(&mut chars)?);
            }
            '`' => word.push_subst(read_backticks(&mut chars)?),
            '#' if !word.started => {
                while let Some(&ch) = chars.peek() {
                    if ch == '\n' {
                        break;
                    }
                    chars.next();
                }
            }
            '\n' => {
                word.flush(&mut tokens);
                tokens.push(Token::Semi);
            }
            c if c.is_whitespace() => word.flush(&mut tokens),
            ';' => {
                word.flush(&mut tokens);
                tokens.push(Token::Semi);
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
                if word.is_bare_fd() {
                    return Err(syntax("file descriptor redirections are not supported"));
                }
                word.flush(&mut tokens);
                if c == '<' {
                    tokens.push(Token::Input);
                } else if chars.peek() == Some(&'>') {
                    chars.next();
                    tokens.push(Token::Append);
                } else {
                    tokens.push(Token::Output);
                }
            }
            other => word.push(other, Mode::Bare),
        }
    }
    word.flush(&mut tokens);
    Ok(tokens)
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

    fn skip_separators(&mut self) {
        while matches!(self.peek(), Some(Token::Semi)) {
            self.pos += 1;
        }
    }

    /// A list of pipelines. Stops before a reserved word in `stops`, before
    /// `)`, or at the end of input.
    fn list(&mut self, stops: &[&str]) -> Result<Script, SandboxError> {
        let mut items = Vec::new();
        loop {
            self.skip_separators();
            match self.peek() {
                None | Some(Token::RParen) => break,
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
                self.skip_separators();
                if matches!(self.peek(), None | Some(Token::RParen)) {
                    return Err(syntax("syntax error: command expected after operator"));
                }
            }
            // After a complete pipeline only a separator, an operator, or the
            // end of a list may follow.
            match self.peek() {
                None | Some(Token::Semi) | Some(Token::RParen) => {}
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
        let mut stages = vec![self.command()?];
        while matches!(self.peek(), Some(Token::Pipe)) {
            self.pos += 1;
            stages.push(self.command()?);
        }
        Ok(Pipeline { stages })
    }

    fn command(&mut self) -> Result<Command, SandboxError> {
        match self.peek() {
            None | Some(Token::Semi | Token::And | Token::Or | Token::Pipe | Token::RParen) => {
                Err(syntax("syntax error: command expected"))
            }
            Some(Token::LParen) => {
                self.pos += 1;
                let body = self.list(&[])?;
                self.expect(&Token::RParen, "')'")?;
                Ok(Command::Subshell(body))
            }
            Some(Token::Word(_)) => match self.keyword().as_deref() {
                Some("if") => self.if_clause(),
                Some("while") => self.while_clause("while"),
                Some("until") => self.while_clause("until"),
                Some("for") => self.for_clause(),
                Some("{") => {
                    self.pos += 1;
                    let body = self.list(&["}"])?;
                    self.expect_keyword("}")?;
                    Ok(Command::Group(body))
                }
                Some(keyword @ ("then" | "elif" | "else" | "fi" | "do" | "done" | "}")) => Err(
                    syntax(format!("syntax error near unexpected token '{keyword}'")),
                ),
                _ if self.is_function_start() => self.function(),
                _ => self.simple(),
            },
            Some(_) => self.simple(),
        }
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

    fn simple(&mut self) -> Result<Command, SandboxError> {
        let mut command = SimpleCommand::default();
        loop {
            match self.peek() {
                Some(Token::Word(word)) => {
                    command.words.push(word.clone());
                    self.pos += 1;
                }
                Some(Token::Input | Token::Output | Token::Append) => {
                    let kind = match self.bump() {
                        Some(Token::Input) => RedirectKind::Input,
                        Some(Token::Output) => RedirectKind::Output,
                        _ => RedirectKind::Append,
                    };
                    match self.bump() {
                        Some(Token::Word(target)) => {
                            command.redirects.push(Redirect { kind, target })
                        }
                        _ => return Err(syntax("redirection needs a file name")),
                    }
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
