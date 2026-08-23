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
        if self.docker_compose_file.is_some() && self.service.is_none() {
            return Err(BondarError::Config(
                "'service' must be specified when using 'dockerComposeFile'".to_string(),
            ));
        }
        Ok(())
    }

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
    candidates.into_iter().find(|p| p.exists())
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

    let raw = fs::read_to_string(&config_path)?;
    let stripped = strip_json_comments(&raw);
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
}
