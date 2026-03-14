use std::env;
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;

use anyhow::{Result, bail};
use serde::Deserialize;

const SANDBOX_CONFIG_FILE: &str = "rustdesk-cli.toml";

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
pub struct SandboxRules {
    #[serde(default)]
    pub allowed_peers: Vec<String>,
    #[serde(default)]
    pub allowed_commands: Vec<String>,
    #[serde(default)]
    pub blocked_paths: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct SandboxConfigFile {
    #[serde(default)]
    sandbox: SandboxRules,
}

#[derive(Debug, Clone)]
pub struct PermissionManager {
    skip_prompts: bool,
    sandbox_rules: Option<SandboxRules>,
}

impl PermissionManager {
    pub fn from_flags(skip_prompts: bool, sandbox_enabled: bool) -> Result<Self> {
        let sandbox_rules = if sandbox_enabled {
            Some(load_sandbox_rules()?)
        } else {
            None
        };

        Ok(Self {
            skip_prompts,
            sandbox_rules,
        })
    }

    pub fn ensure_connect_allowed(&self, peer_id: &str) -> Result<()> {
        if let Some(rules) = &self.sandbox_rules {
            if rules.allowed_peers.is_empty() {
                bail!("sandbox denied connect: no peers are allowed by config");
            }
            if !rules.allowed_peers.iter().any(|allowed| allowed == peer_id) {
                bail!("sandbox denied connect to peer `{peer_id}`");
            }
        }

        self.confirm(&format!("Allow connecting to remote peer `{peer_id}`?"))
    }

    pub fn ensure_exec_allowed(&self, command: &str) -> Result<()> {
        if let Some(rules) = &self.sandbox_rules {
            if rules.allowed_commands.is_empty() {
                bail!("sandbox denied exec: no commands are allowed by config");
            }

            let trimmed = command.trim();
            if !rules
                .allowed_commands
                .iter()
                .any(|allowed| trimmed.starts_with(allowed))
            {
                bail!("sandbox denied exec command `{trimmed}`");
            }

            if let Some(blocked_path) = rules
                .blocked_paths
                .iter()
                .find(|blocked| !blocked.is_empty() && trimmed.contains(blocked.as_str()))
            {
                bail!("sandbox denied exec touching blocked path `{blocked_path}`");
            }
        }

        self.confirm(&format!("Allow remote exec command `{command}`?"))
    }

    pub fn ensure_shell_allowed(&self) -> Result<()> {
        if self.sandbox_rules.is_some() {
            bail!("sandbox denied interactive shell access");
        }

        self.confirm("Allow opening an interactive remote shell?")
    }

    #[allow(dead_code)]
    pub fn ensure_path_allowed(&self, path: &str) -> Result<()> {
        if let Some(rules) = &self.sandbox_rules {
            if let Some(blocked_path) = rules
                .blocked_paths
                .iter()
                .find(|blocked| !blocked.is_empty() && path.contains(blocked.as_str()))
            {
                bail!("sandbox denied path `{path}` because it matches blocked path `{blocked_path}`");
            }
        }

        Ok(())
    }

    fn confirm(&self, prompt: &str) -> Result<()> {
        if self.skip_prompts {
            return Ok(());
        }

        if !io::stdin().is_terminal() {
            bail!(
                "{prompt} Permission prompt requires an interactive terminal; rerun with --dangerously-skip-permissions for automation"
            );
        }

        eprint!("{prompt} [y/N]: ");
        io::stderr().flush()?;

        let mut line = String::new();
        io::stdin().read_line(&mut line)?;
        let answer = line.trim().to_ascii_lowercase();
        if answer == "y" || answer == "yes" {
            return Ok(());
        }

        bail!("operation denied by user")
    }
}

fn load_sandbox_rules() -> Result<SandboxRules> {
    let mut searched = Vec::new();

    for path in sandbox_config_candidates() {
        searched.push(path.display().to_string());
        if !path.exists() {
            continue;
        }

        let raw = fs::read_to_string(&path)?;
        let parsed: SandboxConfigFile = toml::from_str(&raw)?;
        return Ok(parsed.sandbox);
    }

    bail!(
        "sandbox enabled but no `{SANDBOX_CONFIG_FILE}` was found. Searched: {}",
        searched.join(", ")
    )
}

fn sandbox_config_candidates() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    if let Ok(cwd) = env::current_dir() {
        paths.push(cwd.join(SANDBOX_CONFIG_FILE));
    }

    if let Some(home) = env::var_os("HOME").map(PathBuf::from) {
        paths.push(home.join(".config/rustdesk-cli").join(SANDBOX_CONFIG_FILE));
        paths.push(home.join(format!(".{SANDBOX_CONFIG_FILE}")));
    }

    paths
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn sandbox_accepts_allowed_peer() {
        let manager = PermissionManager {
            skip_prompts: true,
            sandbox_rules: Some(SandboxRules {
                allowed_peers: vec!["123".into()],
                ..SandboxRules::default()
            }),
        };

        assert!(manager.ensure_connect_allowed("123").is_ok());
        assert!(manager.ensure_connect_allowed("456").is_err());
    }

    #[test]
    fn sandbox_checks_exec_prefix_and_blocked_paths() {
        let manager = PermissionManager {
            skip_prompts: true,
            sandbox_rules: Some(SandboxRules {
                allowed_commands: vec!["python3".into(), "ls".into()],
                blocked_paths: vec!["/etc".into()],
                ..SandboxRules::default()
            }),
        };

        assert!(manager.ensure_exec_allowed("python3 /tmp/test.py").is_ok());
        assert!(manager.ensure_exec_allowed("rm -rf /tmp").is_err());
        assert!(manager.ensure_exec_allowed("python3 /etc/passwd").is_err());
    }

    #[test]
    fn parses_sandbox_table_from_toml() {
        let parsed: SandboxConfigFile = toml::from_str(
            r#"
            [sandbox]
            allowed_peers = ["308235080"]
            allowed_commands = ["python3", "ls"]
            blocked_paths = ["/etc", "/root"]
            "#,
        )
        .expect("sandbox config should parse");

        assert_eq!(
            parsed.sandbox,
            SandboxRules {
                allowed_peers: vec!["308235080".into()],
                allowed_commands: vec!["python3".into(), "ls".into()],
                blocked_paths: vec!["/etc".into(), "/root".into()],
            }
        );
    }

    #[test]
    fn ensure_path_allowed_denies_blocked_paths() {
        let manager = PermissionManager {
            skip_prompts: true,
            sandbox_rules: Some(SandboxRules {
                blocked_paths: vec!["/root".into()],
                ..SandboxRules::default()
            }),
        };

        assert!(manager.ensure_path_allowed("/tmp/file").is_ok());
        assert!(manager.ensure_path_allowed("/root/secret.txt").is_err());
    }

    #[test]
    fn config_candidates_include_cwd() {
        let cwd = env::current_dir().expect("cwd");
        assert!(sandbox_config_candidates().iter().any(|path| path == &cwd.join(Path::new(SANDBOX_CONFIG_FILE))));
    }
}
