use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::InstallLayoutError;

pub const WINDOWS_INSTANCE_SCHEMA: &str = "aster.windows-instance.v1";

/// Resolved installation settings, not process-global runtime overrides.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowsInstance {
    pub schema: String,
    pub service_prefix: String,
    pub ports: WindowsPorts,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowsPorts {
    pub api: u16,
    pub member: u16,
    pub admin: u16,
    pub blue_api: u16,
    pub blue_member: u16,
    pub blue_admin: u16,
    pub green_api: u16,
    pub green_member: u16,
    pub green_admin: u16,
    pub caddy_admin: u16,
    pub domain_http: u16,
    pub domain_https: u16,
}

impl Default for WindowsInstance {
    fn default() -> Self {
        Self {
            schema: WINDOWS_INSTANCE_SCHEMA.to_owned(),
            service_prefix: String::new(),
            ports: WindowsPorts::default(),
        }
    }
}

impl Default for WindowsPorts {
    fn default() -> Self {
        Self {
            api: 11_080,
            member: 11_081,
            admin: 11_082,
            blue_api: 11_380,
            blue_member: 11_381,
            blue_admin: 11_382,
            green_api: 11_480,
            green_member: 11_481,
            green_admin: 11_482,
            caddy_admin: 2019,
            domain_http: 80,
            domain_https: 443,
        }
    }
}

fn invalid(detail: impl Into<String>) -> InstallLayoutError {
    InstallLayoutError::InvalidMarker(format!("invalid Windows instance: {}", detail.into()))
}

impl WindowsInstance {
    pub fn from_environment(
        mut read: impl FnMut(&str) -> Option<String>,
    ) -> Result<Self, InstallLayoutError> {
        let mut result = Self {
            service_prefix: read("ASTER_SERVICE_PREFIX").unwrap_or_default(),
            ..Self::default()
        };
        let offset: u16 = read("ASTER_PORT_OFFSET").map_or(Ok(0), |value| {
            value
                .parse()
                .map_err(|_| invalid("ASTER_PORT_OFFSET must be an unsigned integer"))
        })?;
        for (name, port) in result.ports.entries_mut() {
            *port = if let Some(value) = read(name) {
                value
                    .parse()
                    .map_err(|_| invalid(format!("{name} must be a port number")))?
            } else {
                port.checked_add(offset)
                    .ok_or_else(|| invalid(format!("{name} overflows with ASTER_PORT_OFFSET")))?
            };
        }
        result.validate()?;
        Ok(result)
    }

    pub fn validate(&self) -> Result<(), InstallLayoutError> {
        if self.schema != WINDOWS_INSTANCE_SCHEMA {
            return Err(invalid("unsupported schema"));
        }
        let prefix = self.service_prefix.as_bytes();
        if prefix.len() > 32
            || (!prefix.is_empty()
                && (!prefix[0].is_ascii_lowercase()
                    || !prefix.iter().all(|byte| {
                        byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-'
                    })))
        {
            return Err(invalid(
                "service prefix must be empty or 1-32 lowercase letters, digits or hyphens, starting with a letter",
            ));
        }
        let mut ports = self.ports;
        let mut unique = BTreeSet::new();
        for (name, port) in ports.entries_mut() {
            if *port == 0 || !unique.insert(*port) {
                return Err(invalid(format!(
                    "{name} must be nonzero and distinct from the other instance ports"
                )));
            }
        }
        Ok(())
    }

    #[must_use]
    pub fn task_path(&self) -> String {
        if self.service_prefix.is_empty() {
            r"\Aster Team\".to_owned()
        } else {
            format!("\\Aster Team\\{}\\", self.service_prefix)
        }
    }
}

impl WindowsPorts {
    fn entries_mut(&mut self) -> [(&'static str, &mut u16); 12] {
        [
            ("ASTER_API_PORT", &mut self.api),
            ("ASTER_MEMBER_PORT", &mut self.member),
            ("ASTER_ADMIN_PORT", &mut self.admin),
            ("ASTER_BLUE_API_PORT", &mut self.blue_api),
            ("ASTER_BLUE_MEMBER_PORT", &mut self.blue_member),
            ("ASTER_BLUE_ADMIN_PORT", &mut self.blue_admin),
            ("ASTER_GREEN_API_PORT", &mut self.green_api),
            ("ASTER_GREEN_MEMBER_PORT", &mut self.green_member),
            ("ASTER_GREEN_ADMIN_PORT", &mut self.green_admin),
            ("ASTER_CADDY_ADMIN_PORT", &mut self.caddy_admin),
            ("ASTER_DOMAIN_HTTP_PORT", &mut self.domain_http),
            ("ASTER_DOMAIN_HTTPS_PORT", &mut self.domain_https),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_environment_preserves_every_legacy_default() {
        let instance = WindowsInstance::from_environment(|_| None).unwrap();
        assert_eq!(instance, WindowsInstance::default());
        assert_eq!(instance.task_path(), "\\Aster Team\\");
    }

    #[test]
    fn resolves_prefix_offset_and_explicit_overrides_once() {
        let instance = WindowsInstance::from_environment(|name| match name {
            "ASTER_SERVICE_PREFIX" => Some("win-smoke".into()),
            "ASTER_PORT_OFFSET" => Some("10000".into()),
            "ASTER_ADMIN_PORT" => Some("25000".into()),
            _ => None,
        })
        .unwrap();
        assert_eq!(instance.task_path(), "\\Aster Team\\win-smoke\\");
        assert_eq!(instance.ports.api, 21080);
        assert_eq!(instance.ports.admin, 25000);
        assert_eq!(instance.ports.blue_api, 21380);
        assert_eq!(instance.ports.green_admin, 21482);
        assert_eq!(instance.ports.caddy_admin, 12019);
        assert_eq!(instance.ports.domain_http, 10080);
        assert_eq!(
            serde_json::from_slice::<WindowsInstance>(&serde_json::to_vec(&instance).unwrap())
                .unwrap(),
            instance
        );
    }

    #[test]
    fn rejects_invalid_prefixes_ports_collisions_and_overflow() {
        for (name, value) in [
            ("ASTER_SERVICE_PREFIX", "../default"),
            ("ASTER_SERVICE_PREFIX", "x'"),
            ("ASTER_PORT_OFFSET", "65535"),
            ("ASTER_API_PORT", "0"),
            ("ASTER_API_PORT", "11081"),
            ("ASTER_API_PORT", "65536"),
            ("ASTER_API_PORT", ""),
        ] {
            assert!(
                WindowsInstance::from_environment(|key| (key == name).then(|| value.to_owned()))
                    .is_err(),
                "{name}={value}"
            );
        }
    }
}
