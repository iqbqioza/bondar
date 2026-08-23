use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{BondarError, Result};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DevContainerConfig {
    #[serde(default)]
    pub name: Option<String>,

    #[serde(default)]
    pub image: Option<String>,

    #[serde(default)]
    pub build: Option<BuildConfig>,

    #[serde(rename = "dockerComposeFile", default)]
    pub docker_compose_file: Option<ComposeFileValue>,

    #[serde(default)]
    pub service: Option<String>,

    #[serde(rename = "runServices", default)]
    pub run_services: Vec<String>,

    #[serde(rename = "workspaceFolder", default)]
    pub workspace_folder: Option<String>,

    #[serde(rename = "workspaceMount", default)]
    pub workspace_mount: Option<String>,

    #[serde(rename = "runArgs", default)]
    pub run_args: Vec<String>,

    #[serde(default)]
    pub mounts: Vec<MountValue>,

    #[serde(rename = "forwardPorts", default)]
    pub forward_ports: Vec<ForwardPort>,

    #[serde(rename = "containerEnv", default)]
    pub container_env: HashMap<String, String>,

    #[serde(rename = "remoteEnv", default)]
    pub remote_env: HashMap<String, String>,

    #[serde(default)]
    pub secrets: Option<HashMap<String, serde_json::Value>>,

    #[serde(rename = "remoteUser", default)]
    pub remote_user: Option<String>,

    #[serde(rename = "containerUser", default)]
    pub container_user: Option<String>,

    #[serde(rename = "overrideCommand", default)]
    pub override_command: Option<bool>,

    #[serde(rename = "customizations", default)]
    pub customizations: Option<serde_json::Value>,

    #[serde(rename = "appPort", default)]
    pub app_port: Option<AppPortValue>,

    #[serde(rename = "privileged", default)]
    pub privileged: Option<bool>,

    #[serde(rename = "capAdd", default)]
    pub cap_add: Vec<String>,

    #[serde(rename = "securityOpt", default)]
    pub security_opt: Vec<String>,

    #[serde(rename = "init", default)]
    pub init: Option<bool>,

    #[serde(default)]
    pub features: Option<HashMap<String, serde_json::Value>>,

    #[serde(rename = "portsAttributes", default)]
    pub ports_attributes: Option<serde_json::Value>,

    #[serde(rename = "otherPortsAttributes", default)]
    pub other_ports_attributes: Option<serde_json::Value>,

    #[serde(rename = "overrideFeatureInstallOrder", default)]
    pub override_feature_install_order: Option<Vec<String>>,

    #[serde(rename = "userEnvProbe", default)]
    pub user_env_probe: Option<String>,

    #[serde(rename = "initializeCommand", default)]
    pub initialize_command: Option<serde_json::Value>,

    #[serde(rename = "onCreateCommand", default)]
    pub on_create_command: Option<serde_json::Value>,

    #[serde(rename = "updateContentCommand", default)]
    pub update_content_command: Option<serde_json::Value>,

    #[serde(rename = "postCreateCommand", default)]
    pub post_create_command: Option<serde_json::Value>,

    #[serde(rename = "postStartCommand", default)]
    pub post_start_command: Option<serde_json::Value>,

    #[serde(rename = "postAttachCommand", default)]
    pub post_attach_command: Option<serde_json::Value>,

    #[serde(rename = "waitFor", default)]
    pub wait_for: Option<String>,

    #[serde(rename = "shutdownAction", default)]
    pub shutdown_action: Option<String>,

    #[serde(rename = "updateRemoteUserUID", default)]
    pub update_remote_user_uid: Option<bool>,

    #[serde(rename = "hostRequirements", default)]
    pub host_requirements: Option<serde_json::Value>,

    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildConfig {
    #[serde(default)]
    pub dockerfile: Option<String>,
    #[serde(default)]
    pub context: Option<String>,
    #[serde(default)]
    pub args: HashMap<String, String>,
    #[serde(default)]
    pub options: Vec<String>,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(rename = "cacheFrom", default)]
    pub cache_from: Option<CacheFromValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CacheFromValue {
    Single(String),
    Multiple(Vec<String>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ComposeFileValue {
    Single(String),
    Multiple(Vec<String>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MountValue {
    String(String),
    Object(MountObject),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MountObject {
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(rename = "type", default)]
    pub mount_type: Option<String>,
    #[serde(default)]
    pub readonly: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ForwardPort {
    Number(u16),
    Text(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AppPortValue {
    Single(PortValue),
    Multiple(Vec<PortValue>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PortValue {
    Number(u16),
    Text(String),
}

impl DevContainerConfig {
    pub fn validate(&self) -> Result<()> {
        if self.image.is_none() && self.build.is_none() && self.docker_compose_file.is_none() {
            return Err(BondarError::Config(
                "Either 'image', 'build' or 'dockerComposeFile' must be specified".to_string(),
            ));
        }
        if self.image.is_some() && self.build.is_some() {
            return Err(BondarError::Config(
                "'image' and 'build' cannot both be specified".to_string(),
            ));
        }
        if let Some(n) = &self.name {
            let trimmed = n.trim();
            if trimmed.is_empty() {
                return Err(BondarError::Config("'name' must not be empty".to_string()));
            }
            if !trimmed.chars().any(|c| c.is_alphanumeric()) {
                return Err(BondarError::Config(
                    "'name' must contain at least one alphanumeric character".to_string(),
                ));
            }
        }
        if let Some(img) = &self.image
            && img.trim().is_empty()
        {
            return Err(BondarError::Config("'image' must not be empty".to_string()));
        }
        if let Some(build) = &self.build
            && let Some(df) = &build.dockerfile
            && df.trim().is_empty()
        {
            return Err(BondarError::Config(
                "'build.dockerfile' must not be empty".to_string(),
            ));
        }
        if self.docker_compose_file.is_some() && self.service.is_none() {
            return Err(BondarError::Config(
                "'service' must be specified when using 'dockerComposeFile'".to_string(),
            ));
        }
        if self.docker_compose_file.is_some() && (self.image.is_some() || self.build.is_some()) {
            return Err(BondarError::Config(
                "'dockerComposeFile' cannot be combined with 'image' or 'build'".to_string(),
            ));
        }
        if self.workspace_mount.is_some() && self.workspace_folder.is_none() {
            return Err(BondarError::Config(
                "'workspaceFolder' must be specified when using 'workspaceMount'".to_string(),
            ));
        }
        if let Some(f) = &self.workspace_folder
            && f.trim().is_empty()
        {
            return Err(BondarError::Config(
                "'workspaceFolder' must not be empty".to_string(),
            ));
        }
        if let Some(f) = &self.docker_compose_file {
            let empty = match f {
                ComposeFileValue::Single(s) => s.trim().is_empty(),
                ComposeFileValue::Multiple(v) => {
                    v.is_empty() || v.iter().any(|s| s.trim().is_empty())
                }
            };
            if empty {
                return Err(BondarError::Config(
                    "'dockerComposeFile' must not be empty".to_string(),
                ));
            }
        }
        if let Some(s) = &self.service
            && s.trim().is_empty()
        {
            return Err(BondarError::Config(
                "'service' must not be empty".to_string(),
            ));
        }
        for key in self.container_env.keys().chain(self.remote_env.keys()) {
            if key.trim().is_empty() {
                return Err(BondarError::Config(
                    "'containerEnv'/'remoteEnv' keys must not be empty".to_string(),
                ));
            }
        }
        Ok(())
    }

    /// Default container workspace folder for image/Dockerfile configurations.
    /// Compose configurations use "/" as their default (handled by callers).
    pub fn workspace_folder_or_default(&self) -> String {
        self.workspace_folder
            .clone()
            .unwrap_or_else(|| "/workspace".to_string())
    }

    pub fn container_name(&self, workspace_path: &Path) -> String {
        if let Some(name) = &self.name {
            let sanitized: String = name
                .chars()
                .map(|c| if c.is_alphanumeric() { c } else { '-' })
                .collect();
            format!("bondar-{sanitized}")
        } else {
            let basename = workspace_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("workspace");
            let sanitized: String = basename
                .chars()
                .map(|c| if c.is_alphanumeric() { c } else { '-' })
                .collect();
            format!("bondar-{sanitized}")
        }
    }
}

pub fn find_config_path(workspace_folder: &Path) -> Option<PathBuf> {
    let candidates = [
        workspace_folder.join(".devcontainer/devcontainer.json"),
        workspace_folder.join(".devcontainer.json"),
    ];
    candidates.into_iter().find(|p| p.is_file())
}

pub fn load_config(
    workspace_folder: &Path,
    override_config: Option<&Path>,
) -> Result<(DevContainerConfig, PathBuf)> {
    let config_path = if let Some(p) = override_config {
        p.to_path_buf()
    } else if let Some(p) = find_config_path(workspace_folder) {
        p
    } else {
        return Err(BondarError::NotFound(format!(
            "devcontainer.json not found in {}",
            workspace_folder.display()
        )));
    };
    let config_path = config_path.canonicalize().map_err(|e| {
        BondarError::NotFound(format!(
            "Cannot resolve config path {}: {e}",
            config_path.display()
        ))
    })?;

    let raw = fs::read_to_string(&config_path)?;
    let raw = raw.strip_prefix('\u{feff}').unwrap_or(&raw);
    let stripped = strip_json_comments(raw);
    let config: DevContainerConfig = serde_json::from_str(&stripped)?;
    config.validate()?;
    Ok((config, config_path))
}

pub fn strip_json_comments(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    let mut in_string = false;
    let mut escaped = false;

    while let Some(c) = chars.next() {
        if in_string {
            output.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }

        if c == '"' {
            in_string = true;
            output.push(c);
            continue;
        }

        if c == '/'
            && let Some(&next) = chars.peek()
        {
            if next == '/' {
                chars.next();
                for nc in chars.by_ref() {
                    if nc == '\n' {
                        output.push('\n');
                        break;
                    }
                }
                continue;
            } else if next == '*' {
                chars.next();
                let mut prev_star = false;
                for nc in chars.by_ref() {
                    if prev_star && nc == '/' {
                        break;
                    }
                    prev_star = nc == '*';
                }
                continue;
            }
        }

        output.push(c);
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_line_comments() {
        let input = r#"{
            // comment
            "image": "ubuntu:22.04" // trailing
        }"#;
        let stripped = strip_json_comments(input);
        let v: serde_json::Value = serde_json::from_str(&stripped).unwrap();
        assert_eq!(v["image"], "ubuntu:22.04");
    }

    #[test]
    fn test_strip_block_comments() {
        let input = r#"{
            /* block */
            "image": "ubuntu:22.04"
        }"#;
        let stripped = strip_json_comments(input);
        let v: serde_json::Value = serde_json::from_str(&stripped).unwrap();
        assert_eq!(v["image"], "ubuntu:22.04");
    }

    #[test]
    fn test_preserve_string_with_slashes() {
        let input = r#"{"image": "http://example.com"}"#;
        let stripped = strip_json_comments(input);
        let v: serde_json::Value = serde_json::from_str(&stripped).unwrap();
        assert_eq!(v["image"], "http://example.com");
    }

    #[test]
    fn test_preserve_double_slash_in_string() {
        let input = r#"{"url": "http://example.com/path//double"}"#;
        let stripped = strip_json_comments(input);
        let v: serde_json::Value = serde_json::from_str(&stripped).unwrap();
        assert_eq!(v["url"], "http://example.com/path//double");
    }

    #[test]
    fn test_preserve_comment_markers_in_string() {
        let input = r#"{"text": "a /* not a comment */ b // also not"}"#;
        let stripped = strip_json_comments(input);
        let v: serde_json::Value = serde_json::from_str(&stripped).unwrap();
        assert_eq!(v["text"], "a /* not a comment */ b // also not");
    }

    #[test]
    fn test_preserve_escaped_quotes_in_string() {
        let input = r#"{"text": "say \"hello\" // not a comment"}"#;
        let stripped = strip_json_comments(input);
        let v: serde_json::Value = serde_json::from_str(&stripped).unwrap();
        assert_eq!(v["text"], "say \"hello\" // not a comment");
    }

    #[test]
    fn test_parse_minimal_config() {
        let json = r#"{"image": "mcr.microsoft.com/devcontainers/base:ubuntu"}"#;
        let cfg: DevContainerConfig = serde_json::from_str(json).unwrap();
        assert_eq!(
            cfg.image.as_deref(),
            Some("mcr.microsoft.com/devcontainers/base:ubuntu")
        );
    }

    #[test]
    fn test_parse_build_config() {
        let json = r#"{
            "build": {
                "dockerfile": "Dockerfile",
                "context": "..",
                "args": {"FOO": "bar"}
            }
        }"#;
        let cfg: DevContainerConfig = serde_json::from_str(json).unwrap();
        let build = cfg.build.unwrap();
        assert_eq!(build.dockerfile.as_deref(), Some("Dockerfile"));
        assert_eq!(build.context.as_deref(), Some(".."));
        assert_eq!(build.args.get("FOO").map(String::as_str), Some("bar"));
    }

    #[test]
    fn test_validate_image_build_conflict() {
        let cfg: DevContainerConfig = serde_json::from_str(
            r#"{"image": "ubuntu:22.04", "build": {"dockerfile": "Dockerfile"}}"#,
        )
        .unwrap();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn test_validate_workspace_mount_requires_folder() {
        let cfg: DevContainerConfig = serde_json::from_str(
            r#"{"image": "ubuntu:22.04", "workspaceMount": "type=bind,source=.,target=/x"}"#,
        )
        .unwrap();
        assert!(cfg.validate().is_err());

        let ok: DevContainerConfig = serde_json::from_str(
            r#"{"image": "ubuntu:22.04", "workspaceMount": "type=bind,source=.,target=/x", "workspaceFolder": "/x"}"#,
        )
        .unwrap();
        assert!(ok.validate().is_ok());
    }

    #[test]
    fn test_validate_compose_conflicts() {
        let cfg: DevContainerConfig = serde_json::from_str(
            r#"{"dockerComposeFile": "docker-compose.yml", "service": "app", "image": "ubuntu"}"#,
        )
        .unwrap();
        assert!(cfg.validate().is_err());

        let no_service: DevContainerConfig =
            serde_json::from_str(r#"{"dockerComposeFile": "docker-compose.yml"}"#).unwrap();
        assert!(no_service.validate().is_err());
    }

    #[test]
    fn test_container_name() {
        let named = DevContainerConfig {
            name: Some("My Dev".to_string()),
            ..Default::default()
        };
        assert_eq!(named.container_name(Path::new("/tmp/x")), "bondar-My-Dev");

        let default = DevContainerConfig::default();
        assert_eq!(
            default.container_name(Path::new("/tmp/my-workspace")),
            "bondar-my-workspace"
        );
    }

    #[test]
    fn test_find_config_path() {
        let dir = std::env::temp_dir().join("bondar-cfg-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".devcontainer")).unwrap();
        assert!(find_config_path(&dir).is_none());

        std::fs::write(dir.join(".devcontainer/devcontainer.json"), "{}").unwrap();
        assert!(find_config_path(&dir).is_some());

        let dir2 = std::env::temp_dir().join("bondar-cfg-test2");
        let _ = std::fs::remove_dir_all(&dir2);
        std::fs::create_dir_all(&dir2).unwrap();
        std::fs::write(dir2.join(".devcontainer.json"), "{}").unwrap();
        let found = find_config_path(&dir2).unwrap();
        assert_eq!(found.file_name().unwrap(), ".devcontainer.json");

        // A directory named devcontainer.json must be ignored (is_file check)
        let dir3 = std::env::temp_dir().join("bondar-cfg-test3");
        let _ = std::fs::remove_dir_all(&dir3);
        std::fs::create_dir_all(dir3.join(".devcontainer/devcontainer.json")).unwrap();
        assert!(find_config_path(&dir3).is_none());

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&dir2);
        let _ = std::fs::remove_dir_all(&dir3);
    }

    #[test]
    fn test_validate_empty_strings() {
        let empty_image: DevContainerConfig = serde_json::from_str(r#"{"image": ""}"#).unwrap();
        assert!(empty_image.validate().is_err());

        let empty_dockerfile: DevContainerConfig =
            serde_json::from_str(r#"{"build": {"dockerfile": ""}}"#).unwrap();
        assert!(empty_dockerfile.validate().is_err());

        let empty_ws: DevContainerConfig =
            serde_json::from_str(r#"{"image": "ubuntu", "workspaceFolder": ""}"#).unwrap();
        assert!(empty_ws.validate().is_err());

        let empty_compose: DevContainerConfig =
            serde_json::from_str(r#"{"dockerComposeFile": "", "service": "app"}"#).unwrap();
        assert!(empty_compose.validate().is_err());

        let empty_service: DevContainerConfig =
            serde_json::from_str(r#"{"dockerComposeFile": "c.yml", "service": ""}"#).unwrap();
        assert!(empty_service.validate().is_err());
    }

    #[test]
    fn test_validate_empty_env_keys() {
        let cfg: DevContainerConfig =
            serde_json::from_str(r#"{"image": "ubuntu", "containerEnv": {"": "value"}}"#).unwrap();
        assert!(cfg.validate().is_err());

        let cfg2: DevContainerConfig =
            serde_json::from_str(r#"{"image": "ubuntu", "remoteEnv": {"": "value"}}"#).unwrap();
        assert!(cfg2.validate().is_err());
    }

    #[test]
    fn test_validate_empty_name() {
        let cfg: DevContainerConfig =
            serde_json::from_str(r#"{"image": "ubuntu", "name": ""}"#).unwrap();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn test_validate_symbolic_name() {
        let cfg: DevContainerConfig =
            serde_json::from_str(r#"{"image": "ubuntu", "name": "!!!"}"#).unwrap();
        assert!(cfg.validate().is_err());

        let ok: DevContainerConfig =
            serde_json::from_str(r#"{"image": "ubuntu", "name": "my-dev"}"#).unwrap();
        assert!(ok.validate().is_ok());
    }
}
