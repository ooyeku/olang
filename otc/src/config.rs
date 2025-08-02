//! Project Configuration Module
//!
//! Provides robust TOML parsing for Olang project configuration files.
//! Replaces the basic string parsing with a proper TOML parser using serde.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Complete Olang project configuration structure
#[derive(Debug, Serialize, Deserialize)]
pub struct OlangProject {
    pub project: ProjectInfo,
    #[serde(default)]
    pub dependencies: HashMap<String, String>,
    #[serde(default)]
    pub build: BuildConfig,
    #[serde(default)]
    pub web: Option<WebConfig>,
    #[serde(default)]
    pub cli: Option<CliConfig>,
    #[serde(default)]
    pub library: Option<LibraryConfig>,
    #[serde(default)]
    pub dev_dependencies: HashMap<String, String>,
}

/// Project metadata and information
#[derive(Debug, Serialize, Deserialize)]
pub struct ProjectInfo {
    pub name: String,
    pub version: String,
    #[serde(rename = "type")]
    pub project_type: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub repository: Option<String>,
    #[serde(default)]
    pub homepage: Option<String>,
    #[serde(default)]
    pub documentation: Option<String>,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub categories: Vec<String>,
}

/// Build configuration options
#[derive(Debug, Serialize, Deserialize)]
pub struct BuildConfig {
    #[serde(default = "default_entry_point")]
    pub entry_point: String,
    #[serde(default = "default_output_dir")]
    pub output_dir: String,
    #[serde(default)]
    pub optimization: Option<String>,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub features: Vec<String>,
    #[serde(default)]
    pub profile: Option<String>,
}

/// Web application configuration
#[derive(Debug, Serialize, Deserialize)]
pub struct WebConfig {
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_static_dir")]
    pub static_dir: String,
    #[serde(default = "default_template_dir")]
    pub template_dir: String,
    #[serde(default)]
    pub middleware: Vec<String>,
    #[serde(default)]
    pub cors: Option<CorsConfig>,
    #[serde(default)]
    pub ssl: Option<SslConfig>,
}

/// CLI application configuration
#[derive(Debug, Serialize, Deserialize)]
pub struct CliConfig {
    pub binary_name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub subcommands: Vec<String>,
    #[serde(default)]
    pub global_flags: Vec<String>,
}

/// Library configuration
#[derive(Debug, Serialize, Deserialize)]
pub struct LibraryConfig {
    pub export_modules: Vec<String>,
    #[serde(default)]
    pub public_api: Vec<String>,
    #[serde(default)]
    pub documentation_modules: Vec<String>,
}

/// CORS configuration for web applications
#[derive(Debug, Serialize, Deserialize)]
pub struct CorsConfig {
    pub allowed_origins: Vec<String>,
    #[serde(default)]
    pub allowed_methods: Vec<String>,
    #[serde(default)]
    pub allowed_headers: Vec<String>,
}

/// SSL configuration for web applications
#[derive(Debug, Serialize, Deserialize)]
pub struct SslConfig {
    pub cert_file: String,
    pub key_file: String,
    #[serde(default)]
    pub ca_file: Option<String>,
}

// Default value functions
fn default_entry_point() -> String {
    "src/main.ol".to_string()
}

fn default_output_dir() -> String {
    "dist".to_string()
}

fn default_port() -> u16 {
    8080
}

fn default_host() -> String {
    "127.0.0.1".to_string()
}

fn default_static_dir() -> String {
    "static".to_string()
}

fn default_template_dir() -> String {
    "templates".to_string()
}

impl Default for BuildConfig {
    fn default() -> Self {
        Self {
            entry_point: default_entry_point(),
            output_dir: default_output_dir(),
            optimization: None,
            target: None,
            features: Vec::new(),
            profile: None,
        }
    }
}

impl OlangProject {
    /// Load project configuration from olang.toml file
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();
        let content = fs::read_to_string(path)
            .with_context(|| format!("Failed to read config file: {}", path.display()))?;
        
        Self::parse_from_str(&content)
            .with_context(|| format!("Failed to parse config file: {}", path.display()))
    }

    /// Parse project configuration from TOML string
    pub fn parse_from_str(content: &str) -> Result<Self> {
        toml::from_str(content)
            .with_context(|| "Invalid TOML syntax in project configuration")
    }

    /// Load configuration from current directory (olang.toml)
    pub fn load_current() -> Result<Self> {
        Self::load_from_file("olang.toml")
    }

    /// Save configuration to file
    #[allow(dead_code)]
    pub fn save_to_file<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let content = toml::to_string_pretty(self)
            .with_context(|| "Failed to serialize project configuration")?;
        
        fs::write(path.as_ref(), content)
            .with_context(|| format!("Failed to write config file: {}", path.as_ref().display()))
    }

    /// Validate project configuration
    pub fn validate(&self) -> Result<()> {
        // Validate project name
        if self.project.name.is_empty() {
            return Err(anyhow::anyhow!("Project name cannot be empty"));
        }

        if !self.project.name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-') {
            return Err(anyhow::anyhow!(
                "Project name '{}' contains invalid characters. Use only alphanumeric, '_', and '-'",
                self.project.name
            ));
        }

        // Validate version format
        if !is_valid_semver(&self.project.version) {
            return Err(anyhow::anyhow!(
                "Invalid version format '{}'. Use semantic versioning (e.g., '1.0.0')",
                self.project.version
            ));
        }

        // Validate project type
        let valid_types = ["application", "library", "web", "cli"];
        if !valid_types.contains(&self.project.project_type.as_str()) {
            return Err(anyhow::anyhow!(
                "Invalid project type '{}'. Valid types: {}",
                self.project.project_type,
                valid_types.join(", ")
            ));
        }

        // Validate entry point exists
        if !Path::new(&self.build.entry_point).exists() {
            return Err(anyhow::anyhow!(
                "Entry point '{}' does not exist",
                self.build.entry_point
            ));
        }

        // Type-specific validation
        match self.project.project_type.as_str() {
            "web" => {
                if self.web.is_none() {
                    return Err(anyhow::anyhow!("Web projects require [web] configuration section"));
                }
                if let Some(web_config) = &self.web {
                    if web_config.port == 0 {
                        return Err(anyhow::anyhow!("Web server port cannot be 0"));
                    }
                }
            }
            "cli" => {
                if self.cli.is_none() {
                    return Err(anyhow::anyhow!("CLI projects require [cli] configuration section"));
                }
            }
            "library" => {
                if self.library.is_none() {
                    return Err(anyhow::anyhow!("Library projects require [library] configuration section"));
                }
                if let Some(lib_config) = &self.library {
                    if lib_config.export_modules.is_empty() {
                        return Err(anyhow::anyhow!("Library projects must specify export_modules"));
                    }
                }
            }
            _ => {}
        }

        Ok(())
    }

    /// Get the effective build entry point
    pub fn get_entry_point(&self) -> &str {
        &self.build.entry_point
    }

    /// Get the effective output directory
    pub fn get_output_dir(&self) -> &str {
        &self.build.output_dir
    }

    /// Check if project has web configuration
    #[allow(dead_code)]
    pub fn is_web_project(&self) -> bool {
        self.project.project_type == "web" || self.web.is_some()
    }

    /// Check if project has CLI configuration
    #[allow(dead_code)]
    pub fn is_cli_project(&self) -> bool {
        self.project.project_type == "cli" || self.cli.is_some()
    }

    /// Check if project is a library
    #[allow(dead_code)]
    pub fn is_library(&self) -> bool {
        self.project.project_type == "library" || self.library.is_some()
    }

    /// Get all dependencies (including dev dependencies)
    #[allow(dead_code)]
    pub fn get_all_dependencies(&self) -> HashMap<String, String> {
        let mut all_deps = self.dependencies.clone();
        all_deps.extend(self.dev_dependencies.clone());
        all_deps
    }

    /// Create a new project configuration with minimal required fields
    #[allow(dead_code)]
    pub fn new(name: String, project_type: String) -> Self {
        Self {
            project: ProjectInfo {
                name,
                version: "0.1.0".to_string(),
                project_type,
                description: None,
                authors: Vec::new(),
                license: None,
                repository: None,
                homepage: None,
                documentation: None,
                keywords: Vec::new(),
                categories: Vec::new(),
            },
            dependencies: HashMap::new(),
            build: BuildConfig::default(),
            web: None,
            cli: None,
            library: None,
            dev_dependencies: HashMap::new(),
        }
    }
}

/// Simple semantic version validation
fn is_valid_semver(version: &str) -> bool {
    let parts: Vec<&str> = version.split('.').collect();
    if parts.len() != 3 {
        return false;
    }
    
    parts.iter().all(|part| part.parse::<u32>().is_ok())
}

/// Check if we're in an Olang project directory
pub fn is_olang_project() -> bool {
    Path::new("olang.toml").exists()
}

/// Ensure we're in an Olang project directory or return error
pub fn ensure_project_directory() -> Result<()> {
    if !is_olang_project() {
        Err(anyhow::anyhow!(
            "Not in an Olang project directory (olang.toml not found)"
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_minimal_config() {
        let toml_content = r#"
[project]
name = "test-project"
version = "1.0.0"
type = "application"
"#;

        let config = OlangProject::parse_from_str(toml_content).unwrap();
        assert_eq!(config.project.name, "test-project");
        assert_eq!(config.project.version, "1.0.0");
        assert_eq!(config.project.project_type, "application");
    }

    #[test]
    fn test_parse_full_web_config() {
        let toml_content = r#"
[project]
name = "web-app"
version = "2.1.0"
type = "web"
description = "A web application"

[dependencies]
http = "1.0"
json = "1.0"

[build]
entry_point = "src/server.ol"
output_dir = "build"

[web]
port = 3000
host = "0.0.0.0"
static_dir = "public"
template_dir = "views"
"#;

        let config = OlangProject::parse_from_str(toml_content).unwrap();
        assert_eq!(config.project.name, "web-app");
        assert!(config.is_web_project());
        assert_eq!(config.web.as_ref().unwrap().port, 3000);
        assert_eq!(config.dependencies.get("http"), Some(&"1.0".to_string()));
    }

    #[test]
    fn test_validation_errors() {
        let mut config = OlangProject::new("".to_string(), "application".to_string());
        assert!(config.validate().is_err()); // Empty name

        config.project.name = "invalid name with spaces".to_string();
        assert!(config.validate().is_err()); // Invalid name characters

        config.project.name = "valid-name".to_string();
        config.project.version = "invalid.version".to_string();
        assert!(config.validate().is_err()); // Invalid version
    }

    #[test]
    fn test_semver_validation() {
        assert!(is_valid_semver("1.0.0"));
        assert!(is_valid_semver("10.20.30"));
        assert!(!is_valid_semver("1.0"));
        assert!(!is_valid_semver("1.0.0.0"));
        assert!(!is_valid_semver("1.0.a"));
    }
} 