use std::{fs, net::IpAddr};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{InstallLayout, Platform};

/// Installation-wide, non-secret database selection. Passwords and CA material
/// live at fixed protected paths, never in service arguments or this document.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "driver", rename_all = "snake_case", deny_unknown_fields)]
pub enum DatabaseConfiguration {
    Sqlcipher {},
    Mariadb {
        host: String,
        port: u16,
        database: String,
        username: String,
        tls: bool,
        #[serde(default)]
        custom_ca: bool,
        max_connections: u32,
    },
}

#[derive(Debug, Error)]
pub enum DatabaseConfigurationError {
    #[error("database configuration is unavailable or unsafe")]
    Io,
    #[error("database configuration is invalid")]
    Invalid,
    #[error("external database installation requires Linux amd64")]
    UnsupportedPlatform,
}

impl DatabaseConfiguration {
    pub fn parse(bytes: &[u8]) -> Result<Self, DatabaseConfigurationError> {
        if bytes.len() > 16 * 1024 {
            return Err(DatabaseConfigurationError::Invalid);
        }
        let config: Self =
            serde_json::from_slice(bytes).map_err(|_| DatabaseConfigurationError::Invalid)?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), DatabaseConfigurationError> {
        if let Self::Mariadb {
            host,
            port,
            database,
            username,
            tls,
            custom_ca,
            max_connections,
        } = self
        {
            let identifier = |value: &str| {
                !value.is_empty()
                    && value.len() <= 64
                    && value
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_')
            };
            let host_valid = !host.is_empty()
                && host.len() <= 253
                && host
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b".-:".contains(&b));
            let loopback = host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback());
            if !host_valid
                || *port == 0
                || !identifier(database)
                || !identifier(username)
                || !(1..=100).contains(max_connections)
                || (!tls && (!loopback || *custom_ca))
            {
                return Err(DatabaseConfigurationError::Invalid);
            }
        }
        Ok(())
    }

    pub fn validate_platform(
        &self,
        platform: Platform,
        arch: &str,
    ) -> Result<(), DatabaseConfigurationError> {
        if self.is_external() && (platform != Platform::Linux || arch != "x86_64") {
            return Err(DatabaseConfigurationError::UnsupportedPlatform);
        }
        Ok(())
    }

    #[must_use]
    pub const fn is_external(&self) -> bool {
        matches!(self, Self::Mariadb { .. })
    }

    /// A missing file means a legacy SQLCipher installation. Corrupt files must
    /// never silently select another database.
    pub fn load(layout: &InstallLayout) -> Result<Self, DatabaseConfigurationError> {
        let path = layout.database_configuration();
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::Sqlcipher {}),
            Ok(metadata)
                if metadata.is_file()
                    && !metadata.file_type().is_symlink()
                    && metadata.len() <= 16 * 1024 =>
            {
                Self::parse(&fs::read(path).map_err(|_| DatabaseConfigurationError::Io)?)
            }
            _ => Err(DatabaseConfigurationError::Io),
        }
    }

    #[must_use]
    pub fn required_files(&self, layout: &InstallLayout) -> Vec<std::path::PathBuf> {
        match self {
            Self::Sqlcipher {} => vec![layout.database_file(), layout.database_key()],
            Self::Mariadb { custom_ca, .. } => {
                let mut files = vec![layout.database_configuration(), layout.database_password()];
                if *custom_ca {
                    files.push(layout.database_ca_certificate());
                }
                files
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_default_and_corrupt_configuration_are_distinct() {
        let directory = tempfile::tempdir().unwrap();
        let layout = InstallLayout::new(directory.path()).unwrap();
        assert_eq!(
            DatabaseConfiguration::load(&layout).unwrap(),
            DatabaseConfiguration::Sqlcipher {}
        );
        fs::create_dir_all(layout.database_configuration().parent().unwrap()).unwrap();
        fs::write(layout.database_configuration(), b"{").unwrap();
        assert!(DatabaseConfiguration::load(&layout).is_err());
        fs::write(layout.database_configuration(), br#"{"driver":"mariadb","host":"127.0.0.1","port":3306,"database":"aster_team","username":"aster_team","tls":false,"max_connections":10}"#).unwrap();
        let required = DatabaseConfiguration::load(&layout)
            .unwrap()
            .required_files(&layout);
        assert!(required.contains(&layout.database_password()));
        assert!(!required.contains(&layout.database_file()));
        assert!(!required.contains(&layout.database_key()));
    }

    #[test]
    fn rejects_unsafe_or_ambiguous_external_configuration() {
        let valid = r#"{"driver":"mariadb","host":"db.internal","port":3306,"database":"aster_team","username":"aster_team","tls":true,"max_connections":10}"#;
        let config = DatabaseConfiguration::parse(valid.as_bytes()).unwrap();
        assert!(config.is_external());
        assert!(config.validate_platform(Platform::Linux, "x86_64").is_ok());
        assert!(
            config
                .validate_platform(Platform::Windows, "x86_64")
                .is_err()
        );
        for invalid in [
            valid.replace("true", "false"),
            valid.replace("3306", "0"),
            valid.replace("\"max_connections\":10", "\"max_connections\":101"),
            valid.replace("\"host\":", "\"password\":\"secret\",\"host\":"),
            valid.replace("db.internal", "db.internal/path"),
        ] {
            assert!(DatabaseConfiguration::parse(invalid.as_bytes()).is_err());
        }
        assert!(
            DatabaseConfiguration::parse(
                valid
                    .replace("db.internal", "127.0.0.1")
                    .replace("true", "false")
                    .as_bytes()
            )
            .is_ok()
        );
        assert!(
            DatabaseConfiguration::parse(br#"{"driver":"sqlcipher","host":"ignored"}"#).is_err()
        );
    }
}
