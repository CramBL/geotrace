use std::env;
use std::io;
use std::path::Path;

#[cfg(unix)]
use rustix::fs::Access;

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
        let companion = installed.with_file_name(COMPANION_FILENAME);
        let resolved = companion
            .canonicalize()
            .map_err(McpConfigError::Companion)?;
        let metadata = resolved.metadata().map_err(McpConfigError::Companion)?;
        if resolved.parent() != companion.parent() || !metadata.is_file() {
            return Err(McpConfigError::CompanionLocation);
        }
        #[cfg(unix)]
        rustix::fs::access(&resolved, Access::EXEC_OK)
            .map_err(|error| McpConfigError::Companion(error.into()))?;

        let alongside_launcher = executable.with_file_name(COMPANION_FILENAME);
        let command = if alongside_launcher.is_absolute()
            && alongside_launcher
                .canonicalize()
                .is_ok_and(|path| path == resolved)
        {
            &alongside_launcher
        } else {
            &companion
        };
        let command = command.to_str().ok_or(McpConfigError::NonUnicodePath)?;
        let json = serde_json::to_string_pretty(&serde_json::json!({
            "mcpServers": { "geotrace": { "command": command } }
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
    #[error("{INSTALL_COMPANION} ({0})")]
    Companion(#[source] io::Error),
    #[error("{INSTALL_COMPANION}")]
    CompanionLocation,
    #[error("Could not locate the GeoTrace executable ({0})")]
    DesktopExecutable(#[source] io::Error),
    #[error("Could not encode the MCP config ({0})")]
    Json(#[source] serde_json::Error),
    #[error("Move GeoTrace to a folder with a Unicode path to copy its MCP config")]
    NonUnicodePath,
}

#[cfg(windows)]
const COMPANION_FILENAME: &str = "geotrace-mcp.exe";
#[cfg(not(windows))]
const COMPANION_FILENAME: &str = "geotrace-mcp";
const INSTALL_COMPANION: &str = "Reinstall GeoTrace with its MCP companion to copy the config";

#[cfg(test)]
mod tests {
    use std::env;
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::{self as unix_fs, PermissionsExt as _};
    use std::path::{Path, PathBuf};

    use rstest::rstest;
    use serde_json::Value;

    use super::{COMPANION_FILENAME, INSTALL_COMPANION, McpConfig};

    #[test]
    fn client_json_preserves_the_absolute_companion_path() {
        let directory = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        let folder_name = r#"GeoTrace \ "install""#;
        #[cfg(windows)]
        let folder_name = "GeoTrace install";
        let installed = directory.path().join(folder_name);
        fs::create_dir(&installed).unwrap();
        let desktop = self::desktop_executable(&installed);
        let companion = installed.join(COMPANION_FILENAME);
        self::write_executable(&companion);

        self::assert_companion_command(CompanionCommandExpectation {
            desktop: &desktop,
            command: &companion,
        });
    }

    #[cfg(unix)]
    #[derive(Clone, Copy)]
    enum LauncherCompanion {
        DifferentInstall,
        SameInstall,
    }

    #[cfg(unix)]
    #[rstest]
    #[case::matching_link(LauncherCompanion::SameInstall)]
    #[case::unrelated_sibling(LauncherCompanion::DifferentInstall)]
    fn the_command_resolves_to_the_running_installation(#[case] sibling: LauncherCompanion) {
        let directory = tempfile::tempdir().unwrap();
        let install = directory.path().join("GeoTrace 0.19");
        let bin = directory.path().join("bin");
        fs::create_dir(&install).unwrap();
        fs::create_dir(&bin).unwrap();
        let desktop = self::desktop_executable(&install);
        let launcher = bin.join(desktop.file_name().unwrap());
        unix_fs::symlink(&desktop, &launcher).unwrap();
        let installed_companion = install.join(COMPANION_FILENAME);
        self::write_executable(&installed_companion);
        let companion_link = bin.join(COMPANION_FILENAME);
        let expected_command = match sibling {
            LauncherCompanion::SameInstall => {
                unix_fs::symlink(&installed_companion, &companion_link).unwrap();
                companion_link
            }
            LauncherCompanion::DifferentInstall => {
                self::write_executable(&companion_link);
                installed_companion.canonicalize().unwrap()
            }
        };
        self::assert_companion_command(CompanionCommandExpectation {
            desktop: &launcher,
            command: &expected_command,
        });
    }

    #[derive(Clone, Copy)]
    enum UnavailableCompanion {
        Directory,
        Missing,
        #[cfg(unix)]
        NonExecutable,
        #[cfg(unix)]
        OtherInstallation,
    }

    #[rstest]
    #[case::missing(UnavailableCompanion::Missing)]
    #[case::directory(UnavailableCompanion::Directory)]
    #[cfg_attr(unix, case::non_executable(UnavailableCompanion::NonExecutable))]
    #[cfg_attr(
        unix,
        case::different_installation(UnavailableCompanion::OtherInstallation)
    )]
    fn copying_requires_an_executable_companion_from_this_installation(
        #[case] unavailable: UnavailableCompanion,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let install = directory.path().join("install");
        fs::create_dir(&install).unwrap();
        let desktop = self::desktop_executable(&install);
        let companion = install.join(COMPANION_FILENAME);
        match unavailable {
            UnavailableCompanion::Missing => {}
            UnavailableCompanion::Directory => fs::create_dir(&companion).unwrap(),
            #[cfg(unix)]
            UnavailableCompanion::NonExecutable => fs::write(&companion, b"file").unwrap(),
            #[cfg(unix)]
            UnavailableCompanion::OtherInstallation => {
                let target = directory.path().join(COMPANION_FILENAME);
                self::write_executable(&target);
                unix_fs::symlink(&target, &companion).unwrap();
            }
        }
        let Err(error) = McpConfig::from_desktop_executable(&desktop) else {
            panic!("expected an unavailable companion");
        };
        assert!(error.to_string().starts_with(INSTALL_COMPANION));
    }

    fn desktop_executable(directory: &Path) -> PathBuf {
        let path = directory.join(format!("geotrace{}", env::consts::EXE_SUFFIX));
        self::write_executable(&path);
        path
    }

    fn write_executable(path: &Path) {
        fs::write(path, b"file").unwrap();
        #[cfg(unix)]
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    struct CompanionCommandExpectation<'a> {
        desktop: &'a Path,
        command: &'a Path,
    }

    fn assert_companion_command(
        CompanionCommandExpectation { desktop, command }: CompanionCommandExpectation<'_>,
    ) {
        let config = McpConfig::from_desktop_executable(desktop)
            .unwrap()
            .into_json();
        let value: Value = serde_json::from_str(&config).unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "mcpServers": { "geotrace": { "command": command.to_str().unwrap() } }
            })
        );
    }
}
