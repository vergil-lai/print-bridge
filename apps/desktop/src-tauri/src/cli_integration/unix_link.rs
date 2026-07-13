use std::{fs, io, path::Path};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LinkState {
    NotInstalled,
    Installed,
    Stale,
    Conflict,
}

pub(super) fn classify_link(target: &Path, expected_source: &Path) -> io::Result<LinkState> {
    let metadata = match fs::symlink_metadata(target) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(LinkState::NotInstalled)
        }
        Err(error) => return Err(error),
    };

    if !metadata.file_type().is_symlink() {
        return Ok(LinkState::Conflict);
    }

    let source = fs::read_link(target)?;
    let source = if source.is_absolute() {
        source
    } else {
        target
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .join(source)
    };

    Ok(if source == expected_source {
        LinkState::Installed
    } else {
        LinkState::Stale
    })
}

#[cfg(test)]
mod tests {
    use super::{classify_link, LinkState};
    use std::{fs, os::unix::fs::symlink};

    #[test]
    fn missing_target_is_not_installed() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("PrintBridge");
        fs::write(&source, b"app").unwrap();

        assert_eq!(
            classify_link(&dir.path().join("print-bridge"), &source).unwrap(),
            LinkState::NotInstalled
        );
    }

    #[test]
    fn matching_link_is_installed() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("PrintBridge");
        let target = dir.path().join("print-bridge");
        fs::write(&source, b"app").unwrap();
        symlink(&source, &target).unwrap();

        assert_eq!(
            classify_link(&target, &source).unwrap(),
            LinkState::Installed
        );
    }

    #[test]
    fn broken_or_different_link_is_stale() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("PrintBridge");
        let target = dir.path().join("print-bridge");
        symlink(dir.path().join("moved-app"), &target).unwrap();

        assert_eq!(classify_link(&target, &source).unwrap(), LinkState::Stale);
    }

    #[test]
    fn regular_file_is_a_conflict() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("PrintBridge");
        let target = dir.path().join("print-bridge");
        fs::write(&source, b"app").unwrap();
        fs::write(&target, b"occupied").unwrap();

        assert_eq!(
            classify_link(&target, &source).unwrap(),
            LinkState::Conflict
        );
    }
}
