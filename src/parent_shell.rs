//! Conservative recognition of actions whose effects cannot persist from a child shell.

pub fn required(command: &str) -> bool {
    let first = command.split_whitespace().next().unwrap_or("");
    matches!(
        first,
        "cd" | "pushd"
            | "popd"
            | "export"
            | "unset"
            | "source"
            | "."
            | "alias"
            | "unalias"
            | "function"
            | "umask"
            | "activate"
    ) || (first.contains('=') && !first.starts_with('=') && !first.contains('/'))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recognizes_common_forms() {
        for c in [
            "cd /tmp",
            "export A=b",
            "source .venv/bin/activate",
            "FOO=bar",
        ] {
            assert!(required(c), "{c}");
        }
        assert!(!required("printf hi"));
    }
}
