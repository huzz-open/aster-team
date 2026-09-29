use super::*;
use std::collections::BTreeMap;

pub(super) fn names(slot: ReleaseSlot) -> [String; 2] {
    [
        format!("aster-control@{}.service", slot.id()),
        format!("aster-runner@{}.service", slot.id()),
    ]
}

pub(super) fn verify(
    layout: &InstallLayout,
    candidate: &ActiveReleaseSlot,
    deadline: Instant,
) -> Result<(), CliFailure> {
    let directory = layout
        .service_registration_root()
        .map_err(|e| failed(e.to_string()))?
        .ok_or_else(|| failed("systemd registration root is unavailable"))?;
    let release = layout.release(&candidate.version);
    for (unit, template) in names(candidate.slot)
        .iter()
        .zip(["aster-control@.service", "aster-runner@.service"])
    {
        let path = directory.join(template);
        let metadata = fs::symlink_metadata(&path).map_err(|e| failed(e.to_string()))?;
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || metadata.nlink() != 1
            || metadata.uid() != 0
            || metadata.mode() & 0o022 != 0
        {
            return Err(failed(
                "candidate service template is not a protected root-owned file",
            ));
        }
        let expected = String::from_utf8(
            read_regular(&release.join("systemd").join(template), 64 * 1024)?.to_vec(),
        )
        .map_err(|_| failed("signed service template is invalid"))?
        .replace(
            "@ASTER_ROOT@",
            layout
                .root()
                .to_str()
                .ok_or_else(|| failed("installation root is not UTF-8"))?,
        );
        if read_regular(&path, 64 * 1024)?.as_slice() != expected.as_bytes() {
            return Err(failed(
                "online startup requires compatible installed service templates",
            ));
        }
        let mut command = Command::new("systemctl");
        command
            .args([
                "--system",
                "--no-ask-password",
                "--no-pager",
                "show",
                "--property=Id,LoadState,FragmentPath,DropInPaths,NeedDaemonReload",
                "--",
            ])
            .arg(unit)
            .env("LC_ALL", "C")
            .env("SYSTEMD_COLORS", "0");
        let bytes = crate::control_process::bounded_output(command, deadline)?;
        validate(
            std::str::from_utf8(&bytes)
                .map_err(|_| failed("service definition observation is invalid"))?,
            unit,
            &path,
        )?;
    }
    Ok(())
}

fn validate(source: &str, unit: &str, path: &Path) -> Result<(), CliFailure> {
    let mut properties = BTreeMap::new();
    for line in source.lines() {
        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| failed("invalid service definition observation"))?;
        if properties.insert(key, value).is_some() {
            return Err(failed("duplicate service definition property"));
        }
    }
    if properties.len() != 5
        || properties.get("Id") != Some(&unit)
        || properties.get("LoadState") != Some(&"loaded")
        || properties.get("FragmentPath").copied() != path.to_str()
        || properties.get("DropInPaths") != Some(&"")
        || properties.get("NeedDaemonReload") != Some(&"no")
    {
        return Err(failed(
            "loaded service definition differs from the verified template",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_overrides_stale_manager_wrong_unit_and_duplicate_properties() {
        let path = Path::new("/etc/systemd/system/aster-control@.service");
        let source = format!(
            "Id=aster-control@green.service\nLoadState=loaded\nFragmentPath={}\nDropInPaths=\nNeedDaemonReload=no\n",
            path.display()
        );
        assert!(validate(&source, "aster-control@green.service", path).is_ok());
        for invalid in [
            source.replace("DropInPaths=", "DropInPaths=/override.conf"),
            source.replace("NeedDaemonReload=no", "NeedDaemonReload=yes"),
            source.replace("green", "blue"),
            format!("{source}Id=aster-control@green.service\n"),
            source.replace("LoadState=loaded", "LoadState=not-found"),
        ] {
            assert!(validate(&invalid, "aster-control@green.service", path).is_err());
        }
        assert_eq!(
            names(ReleaseSlot::Green),
            ["aster-control@green.service", "aster-runner@green.service"]
        );
    }
}
