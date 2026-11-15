use sandbox_shell::arith::evaluate;

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
