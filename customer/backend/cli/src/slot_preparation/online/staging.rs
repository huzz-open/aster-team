//! Select the stopped candidate only after its original intent is durable.
use super::*;
use std::{
    fs,
    os::unix::fs::{PermissionsExt as _, symlink},
};

fn parent(layout: &InstallLayout, path: &Path) -> Result<(), CliFailure> {
    if !path.starts_with(layout.root()) {
        return Err(failed("candidate path escaped the installation"));
    }
    for directory in path
        .parent()
        .ok_or_else(|| failed("candidate path has no parent"))?
        .ancestors()
    {
        let metadata = fs::symlink_metadata(directory).map_err(|e| failed(e.to_string()))?;
        if !metadata.is_dir()
            || metadata.file_type().is_symlink()
            || metadata.permissions().mode() & 0o022 != 0
        {
            return Err(failed("candidate path has an unsafe parent"));
        }
        if directory == layout.root() {
            return Ok(());
        }
    }
    Err(failed("candidate path is outside the installation"))
}

fn points_to(link: &Path, target: &Path) -> Result<bool, CliFailure> {
    let metadata = fs::symlink_metadata(link).map_err(|e| failed(e.to_string()))?;
    if !metadata.file_type().is_symlink() {
        return Err(failed("release pointer is not a symlink"));
    }
    Ok(fs::canonicalize(link).map_err(|e| failed(e.to_string()))?
        == fs::canonicalize(target).map_err(|e| failed(e.to_string()))?)
}

pub(super) fn stage(
    layout: &InstallLayout,
    store: &JournalFile,
    journal: &PreparationJournal,
) -> Result<(), CliFailure> {
    if journal.abort_phase().is_some()
        || journal.startup().is_some()
        || store.load()?.is_some()
        || store.load_preparation()?.as_ref() != Some(journal)
    {
        return Err(failed(
            "candidate staging does not own the original preparation",
        ));
    }
    let intent = journal.intent();
    let previous = layout.release(&intent.previous.version);
    let candidate = intent
        .job
        .candidate_release
        .as_ref()
        .ok_or_else(|| failed("candidate release is missing"))?;
    if intent
        .job
        .target_version
        .as_ref()
        .map(|v| layout.release(v))
        .as_ref()
        != Some(candidate)
    {
        return Err(failed("candidate release differs from its original task"));
    }
    for path in [
        &layout.current(),
        &layout.slot_release(intent.previous.slot.id()),
        &layout.active_slot(),
        &candidate.join("RELEASE.json"),
    ] {
        parent(layout, path)?;
    }
    let active: ActiveReleaseSlot =
        serde_json::from_slice(&read_regular(&layout.active_slot(), 8192)?)
            .map_err(|_| failed("active slot metadata is invalid"))?;
    if active != intent.previous
        || !points_to(&layout.current(), &previous)?
        || !points_to(&layout.slot_release(intent.previous.slot.id()), &previous)?
    {
        return Err(failed("active release changed before candidate staging"));
    }
    let link = layout.slot_release(intent.previous.slot.other().id());
    parent(layout, &link)?;
    match fs::symlink_metadata(&link) {
        Ok(metadata) if !metadata.file_type().is_symlink() => {
            return Err(failed("candidate pointer is not a symlink"));
        }
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(failed(e.to_string())),
    }
    let directory = link
        .parent()
        .ok_or_else(|| failed("candidate pointer has no parent"))?;
    let temporary = directory.join(format!(".online-{}.link", intent.job.id));
    match fs::symlink_metadata(&temporary) {
        Ok(_) if points_to(&temporary, candidate)? => {}
        Ok(_) => {
            return Err(failed(
                "candidate temporary pointer belongs to another target",
            ));
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            symlink(candidate, &temporary).map_err(|e| failed(e.to_string()))?
        }
        Err(e) => return Err(failed(e.to_string())),
    }
    let directory_handle = fs::File::open(directory).map_err(|e| failed(e.to_string()))?;
    directory_handle
        .sync_all()
        .map_err(|e| failed(e.to_string()))?;
    fs::rename(&temporary, &link).map_err(|e| failed(e.to_string()))?;
    directory_handle
        .sync_all()
        .map_err(|e| failed(e.to_string()))?;
    if !points_to(&link, candidate)? {
        return Err(failed("candidate pointer write is uncertain"));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
