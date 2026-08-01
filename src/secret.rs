//! API-key resolution: OPENAI_API_KEY env first, then a 0600 secrets file.
//! Never accepts a key in config.yaml (config gets committed to dotfiles repos).

use crate::dirs;

pub fn resolve_key() -> Result<String, String> {
    if let Ok(k) = std::env::var("OPENAI_API_KEY") {
        if !k.trim().is_empty() {
            return Ok(k.trim().to_string());
        }
    }
    let p = file_path()?;
    if p.exists() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(meta) = std::fs::metadata(&p) {
                let mode = meta.permissions().mode();
                if mode & 0o077 != 0 {
                    return Err(format!(
                        "secrets file {:?} is group/world-readable (mode {:o}); tighten to 0600",
                        p, mode
                    ));
                }
            }
        }
        let s = std::fs::read_to_string(&p)
            .map_err(|e| format!("cannot read secrets file {}: {}", p.display(), e))?;
        for line in s.lines() {
            let line = line.trim();
            if let Some(rest) = line
                .strip_prefix("OPENAI_API_KEY=")
                .or_else(|| line.strip_prefix("openai_api_key="))
            {
                let v = rest.trim().trim_matches('"');
                if !v.is_empty() {
                    return Ok(v.to_string());
                }
            }
        }
    }
    Err(format!(
        "No API key found. Set OPENAI_API_KEY, or run `install -m 600 /dev/null '{}'` and add OPENAI_API_KEY=... with a private editor. Config: {}",
        p.display(),
        dirs::resolve()?.config_file.display()
    ))
}

pub fn file_path() -> Result<std::path::PathBuf, String> {
    Ok(dirs::resolve()?.data_dir.join("secrets"))
}

pub fn mask(k: &str) -> String {
    let chars: Vec<char> = k.chars().collect();
    let n = chars.len();
    if n <= 8 {
        "***".into()
    } else {
        let head: String = chars[..4].iter().collect();
        let tail: String = chars[n - 4..].iter().collect();
        format!("{}...{}", head, tail)
    }
}
