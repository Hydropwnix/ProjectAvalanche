#[macro_export]
macro_rules! println {
    () => {
        $crate::println!("")
    };
    ($($arg:tt)*) => {{
        let line = $crate::text::Text::new(::std::format_args!($($arg)*));
        let bytes = ::std::format!("{line}\n").into_bytes();
        if let Err(e) = $crate::output::line($crate::output::Stream::Out, $crate::output::Keep::Always, bytes) {
            ::std::panic!("failed printing to stdout: {e}");
        }
    }};
}

#[macro_export]
macro_rules! eprintln {
    () => {
        $crate::eprintln!("")
    };
    ($($arg:tt)*) => {{
        let line = $crate::text::Text::new(::std::format_args!($($arg)*));
        let bytes = ::std::format!("{line}\n").into_bytes();
        if let Err(e) = $crate::output::line($crate::output::Stream::Err, $crate::output::Keep::Always, bytes) {
            ::std::panic!("failed printing to stderr: {e}");
        }
    }};
}

pub fn row(fields: &[&dyn std::fmt::Display]) {
    let line = row_line(fields);
    if let Err(e) =
        crate::output::line(crate::output::Stream::Out, crate::output::Keep::Always, format!("{line}\n").into())
    {
        panic!("failed printing to stdout: {e}");
    }
}

pub fn row_line(fields: &[&dyn std::fmt::Display]) -> String {
    let mut line = String::new();
    for (i, f) in fields.iter().enumerate() {
        if i > 0 {
            line.push('\t');
        }
        line.push_str(crate::text::Text::new(f).as_str());
    }
    line
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_row_escapes_its_fields_and_keeps_its_separators() {
        let line = super::row_line(&[&"error", &"a\tb.luau", &3, &"bad\n\u{1b}[2J\u{202e}"]);
        assert_eq!(line, "error\ta\\tb.luau\t3\tbad\\n\\u{1b}[2J\\u{202e}");
    }
}
