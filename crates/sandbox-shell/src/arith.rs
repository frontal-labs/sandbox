//! Integer arithmetic for `$((...))`.
//!
//! The grammar is C's, with the operators bash uses: unary `- + !`, `* / %`,
//! `+ -`, `< > <= >= == !=`, `&&`, `||`, and parentheses. Names read variables,
//! and an unset or empty variable is 0. Values are `i64`. Overflow, division by
//! zero, and malformed input are errors, never panics.

/// Evaluates `expr`. `lookup` returns the value of a variable by name.
pub fn evaluate(expr: &str, lookup: &dyn Fn(&str) -> String) -> Result<i64, String> {
    let tokens = tokenize(expr)?;
    let mut parser = Evaluator {
        tokens,
        pos: 0,
        lookup,
    };
    let value = parser.or()?;
    if parser.pos != parser.tokens.len() {
        return Err(format!("arithmetic syntax error in '{expr}'"));
    }
    Ok(value)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Tok {
    Num(i64),
    Name(String),
    Op(&'static str),
    LParen,
    RParen,
}

const OPERATORS: [&str; 16] = [
    "<=", ">=", "==", "!=", "&&", "||", "<", ">", "+", "-", "*", "/", "%", "!", "$", "=",
];

fn tokenize(expr: &str) -> Result<Vec<Tok>, String> {
    let chars: Vec<char> = expr.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c.is_ascii_digit() {
            let start = i;
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            let text: String = chars[start..i].iter().collect();
            let value = text
                .parse::<i64>()
                .map_err(|_| format!("value too large: {text}"))?;
            tokens.push(Tok::Num(value));
        } else if c == '_' || c.is_ascii_alphabetic() {
            let start = i;
            while i < chars.len() && (chars[i] == '_' || chars[i].is_ascii_alphanumeric()) {
                i += 1;
            }
            tokens.push(Tok::Name(chars[start..i].iter().collect()));
        } else if c == '(' {
            tokens.push(Tok::LParen);
            i += 1;
        } else if c == ')' {
            tokens.push(Tok::RParen);
            i += 1;
        } else {
            let rest: String = chars[i..].iter().collect();
            let op = OPERATORS
                .iter()
                .find(|op| rest.starts_with(**op))
                .ok_or_else(|| format!("arithmetic syntax error: unexpected '{c}'"))?;
            if *op == "=" || *op == "$" {
                return Err(format!("arithmetic syntax error: unsupported '{op}'"));
            }
            tokens.push(Tok::Op(op));
            i += op.len();
        }
    }
    Ok(tokens)
}

struct Evaluator<'a> {
    tokens: Vec<Tok>,
    pos: usize,
    lookup: &'a dyn Fn(&str) -> String,
}

impl Evaluator<'_> {
    fn peek_op(&self) -> Option<&'static str> {
        match self.tokens.get(self.pos) {
            Some(Tok::Op(op)) => Some(op),
            _ => None,
        }
    }

    fn or(&mut self) -> Result<i64, String> {
        let mut value = self.and()?;
        while self.peek_op() == Some("||") {
            self.pos += 1;
            let rhs = self.and()?;
            value = i64::from(value != 0 || rhs != 0);
        }
        Ok(value)
    }

    fn and(&mut self) -> Result<i64, String> {
        let mut value = self.compare()?;
        while self.peek_op() == Some("&&") {
            self.pos += 1;
            let rhs = self.compare()?;
            value = i64::from(value != 0 && rhs != 0);
        }
        Ok(value)
    }

    fn compare(&mut self) -> Result<i64, String> {
        let mut value = self.sum()?;
        while let Some(op) = self.peek_op() {
            if !matches!(op, "<" | ">" | "<=" | ">=" | "==" | "!=") {
                break;
            }
            self.pos += 1;
            let rhs = self.sum()?;
            value = i64::from(match op {
                "<" => value < rhs,
                ">" => value > rhs,
                "<=" => value <= rhs,
                ">=" => value >= rhs,
                "==" => value == rhs,
                _ => value != rhs,
            });
        }
        Ok(value)
    }

    fn sum(&mut self) -> Result<i64, String> {
        let mut value = self.product()?;
        while let Some(op) = self.peek_op() {
            if !matches!(op, "+" | "-") {
                break;
            }
            self.pos += 1;
            let rhs = self.product()?;
            value = if op == "+" {
                value.checked_add(rhs)
            } else {
                value.checked_sub(rhs)
            }
            .ok_or("arithmetic overflow")?;
        }
        Ok(value)
    }

    fn product(&mut self) -> Result<i64, String> {
        let mut value = self.unary()?;
        while let Some(op) = self.peek_op() {
            if !matches!(op, "*" | "/" | "%") {
                break;
            }
            self.pos += 1;
            let rhs = self.unary()?;
            value = match op {
                "*" => value.checked_mul(rhs).ok_or("arithmetic overflow")?,
                _ if rhs == 0 => return Err("division by 0".to_string()),
                "/" => value.checked_div(rhs).ok_or("arithmetic overflow")?,
                _ => value.checked_rem(rhs).ok_or("arithmetic overflow")?,
            };
        }
        Ok(value)
    }

    fn unary(&mut self) -> Result<i64, String> {
        match self.peek_op() {
            Some("-") => {
                self.pos += 1;
                self.unary()?
                    .checked_neg()
                    .ok_or_else(|| "arithmetic overflow".to_string())
            }
            Some("+") => {
                self.pos += 1;
                self.unary()
            }
            Some("!") => {
                self.pos += 1;
                Ok(i64::from(self.unary()? == 0))
            }
            _ => self.primary(),
        }
    }

    fn primary(&mut self) -> Result<i64, String> {
        let token = self
            .tokens
            .get(self.pos)
            .cloned()
            .ok_or("arithmetic syntax error: expression ended early")?;
        self.pos += 1;
        match token {
            Tok::Num(value) => Ok(value),
            Tok::Name(name) => variable_value(&(self.lookup)(&name)),
            Tok::LParen => {
                let value = self.or()?;
                if self.tokens.get(self.pos) != Some(&Tok::RParen) {
                    return Err("arithmetic syntax error: missing ')'".to_string());
                }
                self.pos += 1;
                Ok(value)
            }
            other => Err(format!("arithmetic syntax error near {other:?}")),
        }
    }
}

/// The integer value of a variable's text. Empty text is 0, as in bash.
fn variable_value(text: &str) -> Result<i64, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(0);
    }
    trimmed
        .parse::<i64>()
        .map_err(|_| format!("arithmetic syntax error: '{trimmed}' is not an integer"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval(expr: &str) -> Result<i64, String> {
        evaluate(expr, &|name| match name {
            "x" => "7".to_string(),
            "empty" => String::new(),
            _ => String::new(),
        })
    }

    #[test]
    fn precedence_and_parentheses() {
        assert_eq!(eval("2 + 3 * 4"), Ok(14));
        assert_eq!(eval("(2 + 3) * 4"), Ok(20));
        assert_eq!(eval("10 - 4 - 3"), Ok(3));
        assert_eq!(eval("-7 / 2"), Ok(-3));
        assert_eq!(eval("7 % 3"), Ok(1));
    }

    #[test]
    fn variables_and_unset_names_are_read() {
        assert_eq!(eval("x * 2"), Ok(14));
        assert_eq!(eval("unset + 5"), Ok(5));
        assert_eq!(eval("empty + 1"), Ok(1));
    }

    #[test]
    fn comparisons_and_logic_give_zero_or_one() {
        assert_eq!(eval("3 < 4"), Ok(1));
        assert_eq!(eval("3 >= 4"), Ok(0));
        assert_eq!(eval("2 == 2 && 1 != 0"), Ok(1));
        assert_eq!(eval("0 || !0"), Ok(1));
        assert_eq!(eval("!5"), Ok(0));
    }

    #[test]
    fn errors_are_reported_not_panicked() {
        assert!(eval("1 / 0").unwrap_err().contains("division by 0"));
        assert!(eval("1 +").is_err());
        assert!(eval("(1 + 2").is_err());
        assert!(eval("1 2").is_err());
        assert!(eval("9223372036854775807 + 1")
            .unwrap_err()
            .contains("overflow"));
        assert!(eval("a = 1").is_err());
    }

    #[test]
    fn non_integer_variable_is_an_error() {
        let lookup = |_: &str| "abc".to_string();
        assert!(evaluate("v + 1", &lookup).is_err());
    }
}
