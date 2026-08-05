//! Child-environment policy. Values are never inspected or rendered.

use std::collections::BTreeSet;
use std::process::Command;

pub const COMMON_SECRET_NAMES: &[&str] = &[
    "AWS_ACCESS_KEY_ID",
    "AWS_SECRET_ACCESS_KEY",
    "AWS_SESSION_TOKEN",
    "AZURE_CLIENT_SECRET",
    "AZURE_CLIENT_CERTIFICATE_PASSWORD",
    "CI_JOB_TOKEN",
    "DATABASE_URL",
    "DOCKER_AUTH_CONFIG",
    "GH_TOKEN",
    "GITHUB_TOKEN",
    "GITLAB_TOKEN",
    "GOOGLE_API_KEY",
    "GOOGLE_APPLICATION_CREDENTIALS",
    "KUBECONFIG",
    "MYSQL_PWD",
    "NPM_TOKEN",
    "PGPASSWORD",
    "REDIS_URL",
    "SSH_AUTH_SOCK",
    "VAULT_TOKEN",
];

pub fn apply(command: &mut Command, deny_common: bool, additional: &[String]) {
    command.env_remove("OPENAI_API_KEY");
    command.env_remove("CEREBRAS_API_KEY");
    command.env_remove("DEEPSEEK_API_KEY");
    for (key, _) in std::env::vars_os() {
        let name = key.to_string_lossy();
        if name.starts_with("UHM_PRIVATE_") || name.starts_with("UHM_CONTROL_") {
            command.env_remove(&key);
        }
    }
    if deny_common {
        for name in COMMON_SECRET_NAMES {
            command.env_remove(name);
        }
    }
    for name in additional {
        command.env_remove(name);
    }
}

pub fn exposed_common_names(deny_common: bool, additional: &[String]) -> Vec<&'static str> {
    let additional = additional
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    COMMON_SECRET_NAMES
        .iter()
        .copied()
        .filter(|name| {
            std::env::var_os(name).is_some() && !deny_common && !additional.contains(name)
        })
        .collect()
}
