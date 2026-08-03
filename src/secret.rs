//! Provider-specific API-key resolution: environment first, then a 0600 secrets file.
//! Never accepts a key in config.yaml (config gets committed to dotfiles repos).

use crate::dirs;

pub fn resolve_key(provider: crate::provider::ProviderId) -> Result<String, String> {
    let variable = provider.adapter().credential_env();
    if let Ok(k) = std::env::var(variable) {
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
        if let Some(value) = key_from_text(variable, &s) {
            return Ok(value);
        }
    }
    Err(format!(
        "No API key found for {provider}. Set {variable}, or run `install -m 600 /dev/null '{}'` and add {variable}=... with a private editor. Config: {}",
        p.display(),
        dirs::resolve()?.config_file.display()
    ))
}

fn key_from_text(variable: &str, text: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let rest = line.trim().strip_prefix(&format!("{variable}="))?;
        let value = rest.trim().trim_matches('"');
        (!value.is_empty()).then(|| value.to_string())
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_file_keys_are_provider_isolated() {
        let text = "OPENAI_API_KEY=openai-sentinel\nCEREBRAS_API_KEY=cerebras-sentinel\n";
        assert_eq!(
            key_from_text("OPENAI_API_KEY", text).as_deref(),
            Some("openai-sentinel")
        );
        assert_eq!(
            key_from_text("CEREBRAS_API_KEY", text).as_deref(),
            Some("cerebras-sentinel")
        );
        assert_eq!(key_from_text("OTHER_API_KEY", text), None);
    }
}
