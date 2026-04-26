#[derive(Debug, Clone, Copy)]
pub struct Formatter {
    ensure_trailing_newline: bool,
}

impl Default for Formatter {
    fn default() -> Self {
        Self {
            ensure_trailing_newline: true,
        }
    }
}

impl Formatter {
    pub fn format(&self, input: &str) -> String {
        let normalized = input.replace("\r\n", "\n").replace('\r', "\n");
        let mut output = normalized
            .lines()
            .map(str::trim_end)
            .collect::<Vec<_>>()
            .join("\n");

        if self.ensure_trailing_newline && !output.ends_with('\n') {
            output.push('\n');
        }

        output
    }
}

#[cfg(test)]
mod tests {
    use super::Formatter;

    #[test]
    fn formatter_normalizes_line_endings_and_trailing_space() {
        let formatter = Formatter::default();
        let output = formatter.format("local x = 1  \r\nprint(x)");
        assert_eq!(output, "local x = 1\nprint(x)\n");
    }
}
