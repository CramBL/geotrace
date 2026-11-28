use std::env;
use std::io;
use std::path::Path;

use crate::MCP_COMMAND;

pub(super) struct McpConfig(String);

impl McpConfig {
    pub(super) fn from_current_executable() -> Result<Self, McpConfigError> {
        let executable = env::current_exe().map_err(McpConfigError::DesktopExecutable)?;
        Self::from_desktop_executable(&executable)
    }

    fn from_desktop_executable(executable: &Path) -> Result<Self, McpConfigError> {
        let installed = executable
            .canonicalize()
            .map_err(McpConfigError::DesktopExecutable)?;
        let command = if executable.is_absolute() {
            executable
        } else {
            &installed
        };
        let command = command.to_str().ok_or(McpConfigError::NonUnicodePath)?;
        let json = serde_json::to_string_pretty(&serde_json::json!({
            "mcpServers": {
                "geotrace": {
                    "command": command,
                    "args": [MCP_COMMAND],
                }
            }
        }))
        .map_err(McpConfigError::Json)?;
        Ok(Self(json))
    }

    pub(super) fn into_json(self) -> String {
        self.0
    }
}

#[derive(Debug, thiserror::Error)]
pub(super) enum McpConfigError {
    #[error("Could not locate the GeoTrace executable ({0})")]
    DesktopExecutable(#[source] io::Error),
    #[error("Could not encode the MCP config ({0})")]
    Json(#[source] serde_json::Error),
    #[error("Move GeoTrace to a folder with a Unicode path to copy its MCP config")]
    NonUnicodePath,
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs as unix_fs;
    use std::path::{Path, PathBuf};

    use serde_json::Value;

    use super::{McpConfig, McpConfigError};

    #[test]
    fn client_json_preserves_the_absolute_executable_path() {
        let directory = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        let folder_name = r#"GeoTrace \ "install""#;
        #[cfg(windows)]
        let folder_name = "GeoTrace install";
        let installed = directory.path().join(folder_name);
        fs::create_dir(&installed).unwrap();
        let desktop = self::desktop_executable(&installed);

        self::assert_mcp_command(&desktop);
    }

    #[cfg(unix)]
    #[test]
    fn launcher_symlink_is_preserved_as_the_command() {
        let directory = tempfile::tempdir().unwrap();
        let install = directory.path().join("GeoTrace 0.19");
        let bin = directory.path().join("bin");
        fs::create_dir(&install).unwrap();
        fs::create_dir(&bin).unwrap();
        let desktop = self::desktop_executable(&install);
        let launcher = bin.join(desktop.file_name().unwrap());
        unix_fs::symlink(&desktop, &launcher).unwrap();

        self::assert_mcp_command(&launcher);
    }

    #[test]
    fn missing_executable_returns_desktop_executable_error() {
        let directory = tempfile::tempdir().unwrap();
        let missing = directory.path().join("missing");

        assert!(matches!(
            McpConfig::from_desktop_executable(&missing),
            Err(McpConfigError::DesktopExecutable(_))
        ));
    }

    fn desktop_executable(directory: &Path) -> PathBuf {
        let path = directory.join(format!("geotrace{}", env::consts::EXE_SUFFIX));
        fs::write(&path, b"file").unwrap();
        path
    }

    fn assert_mcp_command(executable: &Path) {
        let config = McpConfig::from_desktop_executable(executable)
            .unwrap()
            .into_json();
        let value: Value = serde_json::from_str(&config).unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "mcpServers": {
                    "geotrace": {
                        "command": executable.to_str().unwrap(),
                        "args": ["mcp"],
                    }
                }
            })
        );
    }
}
