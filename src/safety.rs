//! Advisory local effect detector. It helps users notice consequential work,
//! but it is not a proof of safety and never labels a command "safe".

use crate::action::Effect;

pub const DENY_VERSION: u32 = 3;

const MAX_LEN: usize = 8192;
const MAX_SEGS: usize = 25;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    None,
    Low,
    Network,
    Destructive,
    Irreversible,
}

impl Tier {
    pub fn severity(&self) -> u8 {
        match self {
            Tier::None => 0,
            Tier::Low => 1,
            Tier::Network => 2,
            Tier::Destructive => 3,
            Tier::Irreversible => 4,
        }
    }
}

pub fn higher(a: Tier, b: Tier) -> Tier {
    if a.severity() >= b.severity() {
        a
    } else {
        b
    }
}

pub struct Classification {
    pub tier: Tier,
    pub effects: Vec<Effect>,
    pub reasons: Vec<String>,
}

pub fn classify(cmd: &str) -> Classification {
    if cmd.len() > MAX_LEN {
        return Classification {
            tier: Tier::Irreversible,
            effects: vec![Effect::Unknown],
            reasons: vec!["command too long to classify safely".into()],
        };
    }
    let segs = split_segments(cmd);
    if segs.len() > MAX_SEGS {
        return Classification {
            tier: Tier::Irreversible,
            effects: vec![Effect::Unknown],
            reasons: vec!["too many segments to classify safely".into()],
        };
    }
    let mut tier = Tier::None;
    let mut reasons = Vec::new();
    for seg in &segs {
        let (t, why) = classify_segment(seg);
        tier = higher(tier, t);
        reasons.extend(why);
    }
    let effects = infer_effects(cmd, tier, &reasons);
    Classification {
        tier,
        effects,
        reasons,
    }
}

fn infer_effects(cmd: &str, tier: Tier, reasons: &[String]) -> Vec<Effect> {
    fn add(effects: &mut Vec<Effect>, effect: Effect) {
        if !effects.contains(&effect) {
            effects.push(effect);
        }
    }
    let lower = cmd.to_ascii_lowercase();
    let reason_text = reasons.join(" ").to_ascii_lowercase();
    let mut effects = Vec::new();
    if lower
        .split_whitespace()
        .any(|w| matches!(w, "sudo" | "doas"))
    {
        add(&mut effects, Effect::PrivilegeElevation);
    }
    if reason_text.contains("delete")
        || reason_text.contains("rm ")
        || reason_text.contains("shred")
        || reason_text.contains("drop/truncate")
        || reason_text.contains("disk")
    {
        add(&mut effects, Effect::DeleteLocal);
    }
    if reason_text.contains("kill") || reason_text.contains("process") {
        add(&mut effects, Effect::ProcessControl);
    }
    if reason_text.contains("network") || reason_text.contains("remote") {
        if lower.contains(" push")
            || lower.contains(" install")
            || lower.contains(" delete")
            || lower.contains("--delete")
        {
            add(&mut effects, Effect::RemoteMutation);
        } else {
            add(&mut effects, Effect::NetworkRead);
        }
    }
    if reason_text.contains("overwrite")
        || reason_text.contains("writes")
        || reason_text.contains("redirection")
        || matches!(tier, Tier::Low | Tier::Destructive | Tier::Irreversible) && effects.is_empty()
    {
        add(&mut effects, Effect::WriteLocal);
    }
    if effects.is_empty() {
        add(
            &mut effects,
            if tier == Tier::Network {
                Effect::NetworkRead
            } else {
                Effect::ReadLocal
            },
        );
    }
    effects
}

fn classify_segment(seg: &str) -> (Tier, Vec<String>) {
    let mut reasons = Vec::new();
    let lower = seg.to_lowercase();
    let words: Vec<&str> = seg.split_whitespace().collect();
    let cmd_word = normalized_command(first_command_word(&words));
    let mut tier = Tier::None;

    if matches!(cmd_word, "mkdir" | "touch") {
        tier = higher(tier, Tier::Low);
        reasons.push(format!("{} writes local filesystem metadata", cmd_word));
    }

    if has_dynamic_shell_syntax(seg) {
        tier = higher(tier, Tier::Destructive);
        reasons.push("dynamic shell execution cannot be classified safely".into());
    }

    // ---- irreversible ----
    if cmd_word == "rm" {
        let rec = has_flag_ci(&words, 'r') || words.iter().any(|w| w.starts_with("--rec"));
        let force = has_flag_ci(&words, 'f') || words.iter().any(|w| w.starts_with("--force"));
        if rec && force {
            tier = Tier::Irreversible;
            reasons.push("rm -rf".into());
        } else {
            tier = higher(tier, Tier::Destructive);
            reasons.push("rm deletes files".into());
        }
    }
    if matches!(cmd_word, "rmdir" | "unlink") {
        tier = higher(tier, Tier::Destructive);
        reasons.push(format!("{} deletes files", cmd_word));
    }
    if cmd_word == "dd" && lower.contains("of=/dev/") {
        tier = higher(tier, Tier::Irreversible);
        reasons.push("dd writes to a block device".into());
    } else if cmd_word == "dd" && words.iter().any(|word| word.starts_with("of=")) {
        tier = higher(tier, Tier::Destructive);
        reasons.push("dd writes an output file".into());
    }
    if matches!(cmd_word, "mkfs" | "shred" | "fdisk" | "parted") {
        tier = higher(tier, Tier::Irreversible);
        reasons.push(format!("{} touches disks/partitions", cmd_word));
    }
    if lower.contains(":(){:|:&};:") || lower.contains("fork bomb") {
        tier = higher(tier, Tier::Irreversible);
        reasons.push("fork bomb".into());
    }

    // ---- destructive ----
    if cmd_word == "git" {
        if words.contains(&"reset") && words.iter().any(|w| w.starts_with("--hard")) {
            tier = higher(tier, Tier::Destructive);
            reasons.push("git reset --hard".into());
        }
        if words.contains(&"clean")
            && words
                .iter()
                .any(|w| w.starts_with("-f") || w.starts_with("-d"))
        {
            tier = higher(tier, Tier::Destructive);
            reasons.push("git clean -fd".into());
        }
        if words.contains(&"push") && words.iter().any(|w| w.starts_with("--force") || *w == "-f") {
            tier = higher(tier, Tier::Destructive);
            reasons.push("git push --force".into());
        }
        if words.iter().any(|w| matches!(*w, "restore" | "checkout")) {
            tier = higher(tier, Tier::Destructive);
            reasons.push("git checkout/restore can discard working-tree changes".into());
        }
        if words.iter().any(|w| matches!(*w, "branch" | "stash"))
            && words.iter().any(|w| matches!(*w, "-D" | "drop" | "clear"))
        {
            tier = higher(tier, Tier::Destructive);
            reasons.push("git deletes local state".into());
        }
    }
    if matches!(cmd_word, "chmod" | "chown") && words.iter().any(|w| w.starts_with("-R")) {
        tier = higher(tier, Tier::Destructive);
        reasons.push(format!("{} -R", cmd_word));
    }
    if cmd_word == "kill" {
        tier = higher(tier, Tier::Destructive);
        reasons.push("kill process".into());
    }
    if matches!(cmd_word, "killall" | "pkill") {
        tier = higher(tier, Tier::Destructive);
        reasons.push(cmd_word.into());
    }
    if lower.contains("drop table") || lower.contains("truncate") {
        tier = higher(tier, Tier::Destructive);
        reasons.push("SQL DROP/TRUNCATE".into());
    }
    if matches!(cmd_word, "mv" | "truncate") {
        tier = higher(tier, Tier::Destructive);
        reasons.push(format!("{} can overwrite data", cmd_word));
    }
    if cmd_word == "cp" {
        tier = higher(tier, Tier::Destructive);
        reasons.push("cp can overwrite data".into());
    }
    if cmd_word == "tee"
        || (matches!(cmd_word, "sed" | "perl")
            && words.iter().any(|w| w == &"-i" || w.starts_with("-i")))
    {
        tier = higher(tier, Tier::Destructive);
        reasons.push(format!("{} writes files in place", cmd_word));
    }
    if cmd_word == "find" && words.contains(&"-delete") {
        tier = higher(tier, Tier::Destructive);
        reasons.push("find -delete".into());
    }
    if matches!(cmd_word, "kubectl" | "helm")
        && words.iter().any(|w| matches!(*w, "delete" | "uninstall"))
    {
        tier = higher(tier, Tier::Destructive);
        reasons.push("cluster delete".into());
    }

    // ---- privilege ----
    // `first_command_word` deliberately skips wrappers, so check the actual
    // tokens for privilege-changing wrappers before normalizing the command.
    if words.iter().any(|w| matches!(*w, "sudo" | "doas")) {
        tier = higher(tier, Tier::Destructive);
        reasons.push("privilege escalation".into());
    }

    if has_output_redirection(seg) {
        tier = higher(tier, Tier::Destructive);
        reasons.push("output redirection can overwrite data".into());
    }
    if matches!(cmd_word, "eval" | "source" | ".")
        || (matches!(
            cmd_word,
            "sh" | "bash" | "zsh" | "fish" | "python" | "python3" | "ruby" | "perl"
        ) && words.iter().any(|word| {
            word.starts_with('-')
                && !word.starts_with("--")
                && word.chars().any(|flag| matches!(flag, 'c' | 'e'))
        }))
    {
        tier = higher(tier, Tier::Destructive);
        reasons.push("dynamic code execution cannot be classified safely".into());
    }

    // ---- network ----
    if matches!(cmd_word, "curl" | "wget" | "ssh" | "scp" | "rsync") {
        tier = higher(tier, Tier::Network);
        reasons.push(format!("{} (network)", cmd_word));
    }
    if cmd_word == "curl"
        && words
            .iter()
            .any(|w| matches!(*w, "-o" | "--output") || w.starts_with("--output="))
    {
        tier = higher(tier, Tier::Destructive);
        reasons.push("curl output can overwrite data".into());
    }
    if cmd_word == "git"
        && words
            .iter()
            .any(|w| matches!(*w, "push" | "fetch" | "pull" | "clone"))
    {
        tier = higher(tier, Tier::Network);
        reasons.push("git remote op".into());
    }
    if matches!(
        cmd_word,
        "npm" | "pnpm" | "yarn" | "pip" | "pip3" | "cargo" | "go"
    ) && words
        .iter()
        .any(|w| matches!(*w, "install" | "i" | "add" | "get"))
    {
        tier = higher(tier, Tier::Network);
        reasons.push("package install".into());
    }
    if matches!(
        cmd_word,
        "apt" | "apt-get" | "brew" | "yum" | "dnf" | "pacman"
    ) && words
        .iter()
        .any(|w| matches!(*w, "remove" | "purge" | "uninstall"))
    {
        tier = higher(tier, Tier::Destructive);
        reasons.push("system package removal".into());
    } else if matches!(
        cmd_word,
        "apt" | "apt-get" | "brew" | "yum" | "dnf" | "pacman"
    ) && words.contains(&"install")
    {
        tier = higher(tier, Tier::Network);
        reasons.push("system package op".into());
    }

    // ---- defense-in-depth: lethal substrings ANYWHERE in the segment ----
    // e.g. `find ... -exec rm -rf '{}' +`. Over-warns by design: the model's
    // verdict is advisory, so the local classifier must still catch these even
    // when the command word looks harmless.
    if cmd_word != "rm" {
        for pat in ["rm -rf", "rm -fr", "rm -r -f", "rm -Rf"] {
            if lower.contains(pat) {
                tier = higher(tier, Tier::Irreversible);
                reasons.push(format!("contains '{}'", pat));
            }
        }
    }
    for pat in ["dd of=/dev/", "mkfs", "shred"] {
        if lower.contains(pat) {
            tier = higher(tier, Tier::Irreversible);
            reasons.push(format!("contains '{}'", pat));
        }
    }

    (tier, reasons)
}

/// Skip env assignments (FOO=bar) and prefixes (sudo/nohup/time/exec/env/xargs).
fn first_command_word<'a>(words: &[&'a str]) -> &'a str {
    let mut i = 0;
    while i < words.len() {
        let w = words[i];
        if w.contains('=') && !w.starts_with('-') {
            i += 1;
            continue;
        }
        if matches!(
            w,
            "sudo" | "doas" | "nohup" | "time" | "exec" | "env" | "xargs" | "command"
        ) {
            i += 1;
            continue;
        }
        return w;
    }
    ""
}

fn normalized_command(word: &str) -> &str {
    word.trim_start_matches('\\')
        .rsplit('/')
        .next()
        .unwrap_or(word)
}

/// Output redirects are destructive for gating purposes because `>` truncates
/// before the command runs and `>>` still mutates a file. Quotes and escaped
/// characters are ignored so `echo 'a > b'` remains safe.
fn has_output_redirection(seg: &str) -> bool {
    let mut sq = false;
    let mut dq = false;
    let mut escaped = false;
    let chars: Vec<char> = seg.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if escaped {
            escaped = false;
            i += 1;
            continue;
        }
        if c == '\\' && !sq {
            escaped = true;
            i += 1;
            continue;
        }
        match c {
            '\'' if !dq => sq = !sq,
            '"' if !sq => dq = !dq,
            '>' if !sq && !dq => {
                i += 1;
                if i < chars.len() && chars[i] == '>' {
                    i += 1;
                }
                while i < chars.len() && chars[i].is_whitespace() {
                    i += 1;
                }
                let start = i;
                while i < chars.len()
                    && !chars[i].is_whitespace()
                    && !matches!(chars[i], ';' | '|' | '>')
                {
                    i += 1;
                }
                let target: String = chars[start..i].iter().collect();
                if !matches!(target.as_str(), "/dev/null" | "&1" | "&2") {
                    return true;
                }
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    false
}

fn has_dynamic_shell_syntax(seg: &str) -> bool {
    let mut sq = false;
    let mut dq = false;
    let mut escaped = false;
    let chars: Vec<char> = seg.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if escaped {
            escaped = false;
            i += 1;
            continue;
        }
        if c == '\\' && !sq {
            escaped = true;
            i += 1;
            continue;
        }
        if c == '\'' && !dq {
            sq = !sq;
            i += 1;
            continue;
        }
        if c == '"' && !sq {
            dq = !dq;
            i += 1;
            continue;
        }
        if !sq
            && (c == '`'
                || (c == '$' && chars.get(i + 1) == Some(&'('))
                || (!dq && matches!(c, '(' | ')')))
        {
            return true;
        }
        i += 1;
    }
    false
}

/// Allowlist prefixes apply only to one simple command. A suffix introduced by
/// a shell control operator must never inherit the first command's permission.
#[cfg(test)]
fn is_single_simple_command(cmd: &str) -> bool {
    split_segments(cmd).len() == 1 && !has_output_redirection(cmd) && !has_dynamic_shell_syntax(cmd)
}

fn has_flag_ci(words: &[&str], c: char) -> bool {
    words.iter().any(|w| {
        w.starts_with('-')
            && !w.starts_with("--")
            && w.chars().any(|ch| ch.eq_ignore_ascii_case(&c))
    })
}

/// Split on top-level `;` `|` `&` `&&` `||`, respecting single/double quotes.
fn split_segments(cmd: &str) -> Vec<String> {
    let mut segs = Vec::new();
    let mut cur = String::new();
    let mut sq = false;
    let mut dq = false;
    let chars: Vec<char> = cmd.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\'' if !dq => {
                sq = !sq;
                cur.push(c);
            }
            '"' if !sq => {
                dq = !dq;
                cur.push(c);
            }
            '\\' => {
                cur.push(c);
                if i + 1 < chars.len() {
                    cur.push(chars[i + 1]);
                    i += 1;
                }
            }
            '&' if !sq && !dq && cur.trim_end().ends_with(['>', '<']) => {
                cur.push(c);
            }
            ';' | '|' | '&' | '\n' if !sq && !dq => {
                let t = cur.trim().to_string();
                if !t.is_empty() {
                    segs.push(t);
                }
                cur.clear();
                if (c == '|' || c == '&') && i + 1 < chars.len() && chars[i + 1] == c {
                    i += 1; // consume the second char of && ||
                }
            }
            _ => cur.push(c),
        }
        i += 1;
    }
    let t = cur.trim().to_string();
    if !t.is_empty() {
        segs.push(t);
    }
    segs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_command() {
        assert_eq!(classify("ls -la").tier, Tier::None);
        assert_eq!(classify("git status").tier, Tier::None);
    }

    #[test]
    fn detects_rm_rf_variants() {
        assert_eq!(classify("rm -rf /tmp/x").tier, Tier::Irreversible);
        assert_eq!(classify("rm -r -f /tmp/x").tier, Tier::Irreversible);
        assert_eq!(classify("rm -Rf /tmp/x").tier, Tier::Irreversible);
        assert_eq!(classify("rm file.txt").tier, Tier::Destructive);
    }

    #[test]
    fn destructive_after_pipe() {
        assert_eq!(classify("echo hi | rm -rf /").tier, Tier::Irreversible);
    }

    #[test]
    fn network_and_git_force() {
        assert_eq!(classify("curl https://x.com").tier, Tier::Network);
        assert_eq!(
            classify("git push --force origin main").tier,
            Tier::Destructive
        );
    }

    #[test]
    fn quote_aware_split() {
        assert_eq!(
            classify("echo \"a; b\" | rm -rf /").tier,
            Tier::Irreversible
        );
    }

    #[test]
    fn embedded_rm_rf_via_find_exec() {
        // lethal substring inside a harmless-looking command word
        assert_eq!(
            classify("find . -type d -name build -prune -exec rm -rf '{}' +").tier,
            Tier::Irreversible
        );
    }

    #[test]
    fn destructive_writes_and_privilege_are_gated() {
        assert_eq!(
            classify("echo replaced > important.txt").tier,
            Tier::Destructive
        );
        assert_eq!(
            classify("sudo chmod 777 /etc/shadow").tier,
            Tier::Destructive
        );
        assert_eq!(classify("/bin/rm file.txt").tier, Tier::Destructive);
        assert_eq!(
            classify("find . -name '*.tmp' -delete").tier,
            Tier::Destructive
        );
        assert_eq!(classify("cp source important.txt").tier, Tier::Destructive);
        assert_eq!(classify("sh -c 'rm file.txt'").tier, Tier::Destructive);
    }

    #[test]
    fn quoted_redirect_is_not_a_write() {
        assert_eq!(classify("echo 'a > b'").tier, Tier::None);
        assert_eq!(classify("ls 2>/dev/null").tier, Tier::None);
        assert_eq!(classify("echo x > /dev/null 2>&1").tier, Tier::None);
    }

    #[test]
    fn compound_commands_are_not_simple() {
        assert!(is_single_simple_command("ls -la"));
        assert!(!is_single_simple_command("ls ; rm file.txt"));
        assert!(!is_single_simple_command("ls\nrm file.txt"));
        assert!(!is_single_simple_command("ls $(rm file.txt)"));
        assert_eq!(classify("ls $(rm file.txt)").tier, Tier::Destructive);
    }

    #[test]
    fn adversarial_wrappers_and_remote_mutations_are_detected() {
        assert_eq!(
            classify("env FOO=1 bash -lc 'rm file'").tier,
            Tier::Destructive
        );
        assert_eq!(
            classify("dd if=/dev/zero of=artifact.bin").tier,
            Tier::Destructive
        );
        assert!(classify("rsync --delete src/ host:dst/")
            .effects
            .contains(&Effect::RemoteMutation));
        assert!(classify("git push origin main")
            .effects
            .contains(&Effect::RemoteMutation));
    }
}
