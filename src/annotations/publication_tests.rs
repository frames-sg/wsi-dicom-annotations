use std::fs;

use super::*;

#[test]
fn single_publication_never_replaces_a_destination_created_after_staging() {
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("output.dcm");
    let publication = DicomSinglePublication::new(&destination, "staged.dcm", &[]).unwrap();
    fs::write(publication.staged_file(), b"candidate").unwrap();
    fs::write(&destination, b"winner").unwrap();

    let error = publication.publish().unwrap_err();

    assert_eq!(error.code(), "OUTPUT_PUBLICATION_FAILED");
    assert_eq!(fs::read(&destination).unwrap(), b"winner");
}

#[test]
fn bundle_publication_rechecks_destination_before_rename() {
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("bundle");
    let publication = DicomBundlePublication::new(&destination, &[]).unwrap();
    fs::write(publication.staging_path().join("pm-0001.dcm"), b"candidate").unwrap();
    fs::create_dir(&destination).unwrap();

    let error = publication.publish().unwrap_err();

    assert_eq!(error.code(), "OUTPUT_EXISTS");
    assert!(destination.is_dir());
    assert!(fs::read_dir(destination).unwrap().next().is_none());
}

#[test]
fn bundle_publication_publishes_a_synced_staging_directory() {
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("bundle");
    let publication = DicomBundlePublication::new(&destination, &[]).unwrap();
    fs::write(publication.staging_path().join("manifest.json"), b"{}").unwrap();

    let published = publication.publish().unwrap();

    assert_eq!(
        published,
        directory.path().canonicalize().unwrap().join("bundle")
    );
    assert_eq!(fs::read(published.join("manifest.json")).unwrap(), b"{}");
}

#[cfg(unix)]
#[test]
fn publication_rejects_symlinked_parent_escape_and_existing_aliases() {
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir().unwrap();
    let protected = directory.path().join("input.zarr");
    let apparent_parent = directory.path().join("apparent");
    fs::create_dir(&protected).unwrap();
    symlink(&protected, &apparent_parent).unwrap();
    let error = match DicomSinglePublication::new(
        &apparent_parent.join("output.dcm"),
        "staged.dcm",
        &[&protected],
    ) {
        Ok(_) => panic!("output inside a protected directory should be rejected"),
        Err(error) => error,
    };
    assert_eq!(error.code(), "UNSAFE_OUTPUT_PATH");

    let source = directory.path().join("source.dcm");
    let hardlink = directory.path().join("hardlink.dcm");
    let symlink_path = directory.path().join("symlink.dcm");
    fs::write(&source, b"source").unwrap();
    fs::hard_link(&source, &hardlink).unwrap();
    symlink(&source, &symlink_path).unwrap();
    for destination in [&hardlink, &symlink_path] {
        let error = match DicomSinglePublication::new(destination, "staged.dcm", &[&source]) {
            Ok(_) => panic!("an existing alias should be rejected"),
            Err(error) => error,
        };
        assert_eq!(error.code(), "OUTPUT_EXISTS");
    }
}
