use std::fs::{self, OpenOptions};
use std::io;
use std::path::Path;

fn link(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    metadata.file_type().is_symlink()
}
fn owned_name(name: &str) -> bool {
    matches!(
        name,
        "identity.txt"
            | "session.lock"
            | "session.txt"
            | "session-end.txt"
            | "panic.log"
            | "application.log"
            | "application.1.log"
            | "application.2.log"
            | "application.3.log"
            | "crash.dmp"
            | "native-crash.txt"
            | "process-exit.txt"
            | "collector-error.txt"
    )
}

// Delete only recognized, inactive sessions and their flat owned files; never recurse.
pub(super) fn prune(root: &Path, count_limit: usize, byte_limit: u64) -> io::Result<()> {
    let mut sessions = Vec::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with("session-")
            || !name[8..].bytes().all(|c| c.is_ascii_digit() || c == b'-')
        {
            continue;
        }
        let metadata = fs::symlink_metadata(entry.path())?;
        if !metadata.is_dir() || link(&metadata) {
            continue;
        }
        let directory = entry.path();
        let mut files = Vec::new();
        let mut bytes = 0u64;
        let mut owned = true;
        for file in fs::read_dir(&directory)? {
            let file = file?;
            let metadata = fs::symlink_metadata(file.path())?;
            if !metadata.is_file()
                || link(&metadata)
                || !owned_name(&file.file_name().to_string_lossy())
            {
                owned = false;
                break;
            }
            bytes = bytes.saturating_add(metadata.len());
            files.push(file.path());
        }
        if !owned
            || fs::read_to_string(directory.join("identity.txt"))
                .ok()
                .as_deref()
                != Some(super::SESSION_MARKER)
        {
            continue;
        }
        sessions.push((name, directory, files, bytes));
    }
    sessions.sort_by(|a, b| b.0.cmp(&a.0));
    let mut bytes = 0u64;
    for (index, (_, directory, files, size)) in sessions.into_iter().enumerate() {
        bytes = bytes.saturating_add(size);
        if index < count_limit && bytes <= byte_limit {
            continue;
        }
        let Ok(lock) = OpenOptions::new()
            .read(true)
            .write(true)
            .open(directory.join("session.lock"))
        else {
            continue;
        };
        if lock.try_lock().is_err() {
            continue;
        }
        for file in files {
            fs::remove_file(file)?;
        }
        drop(lock);
        fs::remove_dir(directory)?;
        bytes = bytes.saturating_sub(size);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pruning_preserves_active_sessions_and_unowned_data() -> io::Result<()> {
        let root = std::env::temp_dir().join(format!(
            "tsan-retention-{}-{}",
            std::process::id(),
            super::super::now()
        ));
        fs::create_dir(&root)?;
        for i in 0..5 {
            let d = root.join(format!("session-{i:020}-123"));
            fs::create_dir(&d)?;
            fs::write(d.join("identity.txt"), super::super::SESSION_MARKER)?;
            fs::write(d.join("session.lock"), "")?;
        }
        let active = root.join("session-00000000000000000000-123");
        let _active_lock = super::super::session_lock(&active)?;
        let unowned = root.join("session-00000000000000000001-123");
        fs::write(unowned.join("user-report.txt"), "preserve")?;
        prune(&root, 1, u64::MAX)?;
        assert!(active.exists());
        assert!(unowned.join("user-report.txt").exists());
        assert!(!root.join("session-00000000000000000002-123").exists());
        assert!(root.join("session-00000000000000000004-123").exists());
        drop(_active_lock);
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
