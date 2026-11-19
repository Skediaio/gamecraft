//! What `project.query` accepts: exactly one `SELECT` statement.

/// Checks that `sql` is one `SELECT` statement and returns it without a trailing `;`.
///
/// Refused, each with a sentence that says why: an empty query, more than one statement, a
/// statement that does not start with `SELECT` (the index is read-only: `INSERT`, `UPDATE`,
/// `DELETE`, `CREATE`, `DROP`, `PRAGMA`, `ATTACH`, `WITH`, ... are refused), and any use of
/// `load_extension`. Quotes (`'...'`, `"..."`, `` `...` ``, `[...]`) and comments (`--`,
/// `/* */`) are skipped when looking for a `;`.
pub fn check_select(sql: &str) -> Result<&str, String> {
    let trimmed = sql.trim();
    let body = trimmed.strip_suffix(';').unwrap_or(trimmed).trim_end();
    if body.is_empty() {
        return Err("`sql` is empty; give one SELECT statement".into());
    }
    let mut chars = body.char_indices().peekable();
    let mut first_word = String::new();
    let mut word_done = false;
    while let Some((_, c)) = chars.next() {
        match c {
            '\'' | '"' | '`' | '[' => {
                word_done = true;
                let close = if c == '[' { ']' } else { c };
                let mut closed = false;
                while let Some((_, d)) = chars.next() {
                    if d == close {
                        // A doubled quote inside a string is a quote, not the end.
                        if close != ']' && chars.peek().is_some_and(|(_, n)| *n == close) {
                            chars.next();
                            continue;
                        }
                        closed = true;
                        break;
                    }
                }
                if !closed {
                    return Err(format!(
                        "the query has a {c} that is never closed; close it, or check the quoting"
                    ));
                }
            }
            '-' if chars.peek().is_some_and(|(_, n)| *n == '-') => {
                for (_, d) in chars.by_ref() {
                    if d == '\n' {
                        break;
                    }
                }
                if !first_word.is_empty() {
                    word_done = true;
                }
            }
            '/' if chars.peek().is_some_and(|(_, n)| *n == '*') => {
                chars.next();
                let mut prev = ' ';
                for (_, d) in chars.by_ref() {
                    if prev == '*' && d == '/' {
                        break;
                    }
                    prev = d;
                }
                if !first_word.is_empty() {
                    word_done = true;
                }
            }
            ';' => {
                return Err(
                    "the query holds more than one statement; give one SELECT statement".into(),
                );
            }
            c if !word_done && c.is_ascii_alphabetic() => first_word.push(c),
            c if !word_done && c.is_whitespace() => {
                if !first_word.is_empty() {
                    word_done = true;
                }
            }
            _ => word_done = true,
        }
    }
    if !first_word.eq_ignore_ascii_case("select") {
        let shown = if first_word.is_empty() {
            body.chars().take(20).collect::<String>()
        } else {
            first_word.to_uppercase()
        };
        return Err(format!(
            "only one SELECT statement is allowed, and this one starts with `{shown}`; the \
             index is read-only (it is rebuilt from the files), so change the world with \
             commands instead"
        ));
    }
    if body.to_ascii_lowercase().contains("load_extension") {
        return Err("`load_extension` is not allowed in a query of the index".into());
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::check_select;

    #[test]
    fn one_select_passes_and_everything_else_is_refused() {
        assert_eq!(check_select(" select 1 ; ").unwrap(), "select 1");
        assert!(check_select("SELECT name FROM entities WHERE name = 'a;b'").is_ok());
        assert!(check_select("/* hi */ -- there\n SELECT 1").is_ok());
        assert!(check_select("SELECT \"x;y\" FROM [t;u]").is_ok());
        for (bad, why) in [
            ("", "empty"),
            (";", "empty"),
            ("SELECT 1; DELETE FROM entities", "more than one"),
            ("DELETE FROM entities", "`DELETE`"),
            ("insert into entities values (1)", "`INSERT`"),
            ("DROP TABLE entities", "`DROP`"),
            ("PRAGMA query_only = 0", "`PRAGMA`"),
            ("ATTACH 'x' AS y", "`ATTACH`"),
            ("WITH a AS (SELECT 1) DELETE FROM entities", "`WITH`"),
            ("SELECT load_extension('evil')", "load_extension"),
            ("SELECT 'unclosed", "never closed"),
            ("(SELECT 1)", "starts with"),
        ] {
            let e = check_select(bad).unwrap_err();
            assert!(e.contains(why), "{bad:?}: {e}");
        }
    }
}
