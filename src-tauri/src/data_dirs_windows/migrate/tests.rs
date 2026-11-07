use super::*;
use std::time::{Duration, SystemTime};

fn write(path: &Path, bytes: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

fn read(path: &Path) -> Vec<u8> {
    fs::read(path).unwrap()
}

/// A Roaming folder holding the three heavy folders and the small files that
/// stay, and an empty Local folder next to it.
fn profile() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let roaming = root.path().join("Roaming").join("app");
    let local = root.path().join("Local").join("app");
    write(&roaming.join("settings_store.json"), b"{}");
    write(&roaming.join("history.db"), b"history");
    write(&roaming.join("meetings.db"), b"meetings");
    write(
        &roaming.join("models").join("parakeet").join("a.gguf"),
        b"model-a",
    );
    write(&roaming.join("models").join("vocab.json"), b"{}");
    write(&roaming.join("meetings").join("7").join("mic.wav"), b"mic");
    write(
        &roaming.join("meetings").join("7").join("system.wav"),
        b"system",
    );
    write(&roaming.join("recordings").join("murmur-1.wav"), b"one");
    (root, roaming, local)
}

fn set_modified(path: &Path, age: Duration) {
    let time = SystemTime::now() - age;
    fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(time)
        .unwrap();
}

#[test]
fn a_missing_folder_has_nothing_to_move() {
    let root = tempfile::tempdir().unwrap();
    let outcome = move_folder(&root.path().join("old"), &root.path().join("new"));
    assert_eq!(outcome, Moved::Done);
    assert!(!root.path().join("new").exists());
}

#[test]
fn every_heavy_folder_moves_and_the_small_files_stay() {
    let (_root, roaming, local) = profile();

    let layout = resolve(&roaming, &local);

    assert_eq!(layout.models_parent(), local);
    assert_eq!(layout.meetings(), local.join("meetings"));
    assert_eq!(layout.recordings(), local.join("recordings"));
    assert_eq!(layout.roaming(), roaming);
    assert_eq!(
        read(&local.join("models/parakeet/a.gguf")),
        b"model-a".to_vec()
    );
    assert_eq!(read(&local.join("meetings/7/system.wav")), b"system");
    assert_eq!(read(&local.join("recordings/murmur-1.wav")), b"one");
    for moved in ["models", "meetings", "recordings"] {
        assert!(!roaming.join(moved).exists(), "{moved} is left in Roaming");
    }
    for stays in ["settings_store.json", "history.db", "meetings.db"] {
        assert!(roaming.join(stays).is_file(), "{stays} left Roaming");
    }
}

#[test]
fn a_second_launch_changes_nothing() {
    let (_root, roaming, local) = profile();
    let first = resolve(&roaming, &local);
    write(&local.join("recordings/murmur-2.wav"), b"two");

    let second = resolve(&roaming, &local);

    assert_eq!(first, second);
    assert_eq!(read(&local.join("recordings/murmur-2.wav")), b"two");
    assert_eq!(read(&local.join("recordings/murmur-1.wav")), b"one");
}

#[test]
fn an_empty_folder_in_the_way_does_not_stop_the_move() {
    let (_root, roaming, local) = profile();
    fs::create_dir_all(local.join("recordings")).unwrap();

    assert_eq!(
        move_folder(&roaming.join("recordings"), &local.join("recordings")),
        Moved::Done
    );

    assert_eq!(read(&local.join("recordings/murmur-1.wav")), b"one");
    assert!(!roaming.join("recordings").exists());
}

#[test]
fn a_launch_killed_halfway_carries_on_where_it_stopped() {
    let (_root, roaming, local) = profile();
    // The first launch copied a.gguf, was killed while copying the next file
    // and left its part file, and deleted nothing.
    write(&local.join("models/parakeet/a.gguf"), b"model-a");
    write(&local.join("models/vocab.json.moving"), b"{");

    let outcome = move_folder(&roaming.join("models"), &local.join("models"));

    assert_eq!(outcome, Moved::Done);
    assert_eq!(read(&local.join("models/vocab.json")), b"{}");
    assert_eq!(
        read(&local.join("models/parakeet/a.gguf")),
        b"model-a".to_vec()
    );
    assert!(!local.join("models/vocab.json.moving").exists());
    assert!(!roaming.join("models").exists());
}

#[test]
fn a_part_file_is_never_taken_for_the_file() {
    let (_root, roaming, local) = profile();
    write(&local.join("recordings/murmur-1.wav.moving"), b"on");

    move_folder(&roaming.join("recordings"), &local.join("recordings"));

    assert_eq!(read(&local.join("recordings/murmur-1.wav")), b"one");
}

#[test]
fn files_the_old_folder_gained_after_a_failed_launch_are_moved_too() {
    let (_root, roaming, local) = profile();
    write(&local.join("recordings/murmur-1.wav"), b"one");
    // Murmur kept running from Roaming after that launch.
    write(&roaming.join("recordings/murmur-9.wav"), b"nine");

    move_folder(&roaming.join("recordings"), &local.join("recordings"));

    assert_eq!(read(&local.join("recordings/murmur-9.wav")), b"nine");
    assert!(!roaming.join("recordings").exists());
}

#[test]
fn a_file_grown_in_the_old_folder_replaces_its_older_copy() {
    let (_root, roaming, local) = profile();
    write(&local.join("recordings/murmur-1.wav"), b"o");
    set_modified(
        &local.join("recordings/murmur-1.wav"),
        Duration::from_secs(600),
    );

    move_folder(&roaming.join("recordings"), &local.join("recordings"));

    assert_eq!(read(&local.join("recordings/murmur-1.wav")), b"one");
}

#[test]
fn a_newer_file_in_the_new_folder_wins_and_the_old_one_is_kept() {
    let (_root, roaming, local) = profile();
    write(&local.join("recordings/murmur-1.wav"), b"edited later");
    set_modified(
        &roaming.join("recordings/murmur-1.wav"),
        Duration::from_secs(600),
    );

    let outcome = move_folder(&roaming.join("recordings"), &local.join("recordings"));

    assert_eq!(outcome, Moved::Done);
    assert_eq!(
        read(&local.join("recordings/murmur-1.wav")),
        b"edited later"
    );
    assert_eq!(read(&roaming.join("recordings/murmur-1.wav")), b"one");
}

#[test]
fn the_folder_never_loses_a_file_when_a_target_is_not_a_file() {
    let (_root, roaming, local) = profile();
    fs::create_dir_all(local.join("recordings/murmur-1.wav")).unwrap();

    let outcome = move_folder(&roaming.join("recordings"), &local.join("recordings"));

    assert_eq!(outcome, Moved::Done);
    assert_eq!(read(&roaming.join("recordings/murmur-1.wav")), b"one");
}

#[test]
fn a_file_in_the_way_of_the_new_folder_keeps_the_old_one_in_use() {
    let (_root, roaming, local) = profile();
    write(&local.join("models"), b"not a folder");

    let outcome = move_folder(&roaming.join("models"), &local.join("models"));

    assert!(matches!(outcome, Moved::Kept(_)), "{outcome:?}");
    assert_eq!(read(&roaming.join("models/vocab.json")), b"{}");
}

#[test]
fn a_file_in_place_of_the_old_folder_is_left_alone() {
    let root = tempfile::tempdir().unwrap();
    let old = root.path().join("old");
    write(&old, b"a file");

    let outcome = move_folder(&old, &root.path().join("new"));

    assert!(matches!(outcome, Moved::Kept(_)), "{outcome:?}");
    assert_eq!(read(&old), b"a file");
}

#[test]
fn a_folder_that_cannot_be_created_keeps_the_old_one_in_use() {
    let (root, roaming, _local) = profile();
    // The parent of the new folder is a file.
    write(&root.path().join("blocked"), b"file");

    let layout = resolve(&roaming, &root.path().join("blocked").join("app"));

    assert_eq!(layout, Layout::in_roaming(&roaming));
    assert_eq!(read(&roaming.join("meetings/7/mic.wav")), b"mic");
}

#[test]
fn the_same_folder_for_both_moves_nothing() {
    let (_root, roaming, _local) = profile();

    assert_eq!(resolve(&roaming, &roaming), Layout::in_roaming(&roaming));
    assert_eq!(read(&roaming.join("recordings/murmur-1.wav")), b"one");
}

#[test]
fn a_folder_that_stays_in_roaming_does_not_hold_back_the_others() {
    let (_root, roaming, local) = profile();
    write(&local.join("models"), b"not a folder");

    let layout = resolve(&roaming, &local);

    assert_eq!(layout.models_parent(), roaming);
    assert_eq!(layout.meetings(), local.join("meetings"));
    assert_eq!(layout.recordings(), local.join("recordings"));
    assert_eq!(layout.set_aside_dirs(), vec![local.clone()]);
}

#[test]
fn what_is_set_aside_lands_beside_the_folder_it_comes_from() {
    let (_root, roaming, local) = profile();

    assert!(Layout::in_roaming(&roaming).set_aside_dirs().is_empty());
    assert_eq!(resolve(&roaming, &local).set_aside_dirs(), vec![local]);
}

#[cfg(unix)]
mod unix {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// Permission bits do not stop the superuser.
    fn can_deny_reads(path: &Path) -> bool {
        fs::set_permissions(path, fs::Permissions::from_mode(0o000)).unwrap();
        let denied = fs::File::open(path).is_err();
        fs::set_permissions(path, fs::Permissions::from_mode(0o644)).unwrap();
        denied
    }

    #[test]
    fn a_copy_that_fails_deletes_nothing_and_keeps_the_old_folder_in_use() {
        let (_root, roaming, local) = profile();
        // The new folder is not empty, so the move copies instead of renaming.
        write(&local.join("meetings/1/other.wav"), b"x");
        let locked = roaming.join("meetings/7/system.wav");
        if !can_deny_reads(&locked) {
            return;
        }
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();

        let layout = resolve(&roaming, &local);

        fs::set_permissions(&locked, fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(layout.meetings(), roaming.join("meetings"));
        assert_eq!(read(&roaming.join("meetings/7/mic.wav")), b"mic");
        assert_eq!(read(&locked), b"system");
        assert!(!local.join("meetings/7/system.wav.moving").exists());
        assert!(!local.join("meetings/7/system.wav").exists());

        // The lock goes away: the next launch finishes the move.
        let layout = resolve(&roaming, &local);
        assert_eq!(layout.meetings(), local.join("meetings"));
        assert_eq!(read(&local.join("meetings/7/system.wav")), b"system");
        assert!(!roaming.join("meetings").exists());
    }

    #[test]
    fn links_are_left_where_they_are() {
        let (_root, roaming, local) = profile();
        write(&local.join("recordings/keep.wav"), b"k");
        std::os::unix::fs::symlink(
            roaming.join("settings_store.json"),
            roaming.join("recordings/link.wav"),
        )
        .unwrap();

        let outcome = move_folder(&roaming.join("recordings"), &local.join("recordings"));

        assert_eq!(outcome, Moved::Done);
        assert!(roaming.join("recordings/link.wav").is_symlink());
        assert!(!local.join("recordings/link.wav").exists());
        assert_eq!(read(&local.join("recordings/murmur-1.wav")), b"one");
        assert_eq!(read(&roaming.join("settings_store.json")), b"{}");
    }

    #[test]
    fn a_link_in_place_of_the_old_folder_is_left_alone() {
        let (root, roaming, local) = profile();
        let elsewhere = root.path().join("elsewhere");
        write(&elsewhere.join("a.wav"), b"a");
        fs::remove_dir_all(roaming.join("recordings")).unwrap();
        std::os::unix::fs::symlink(&elsewhere, roaming.join("recordings")).unwrap();

        let layout = resolve(&roaming, &local);

        assert_eq!(layout.recordings(), roaming.join("recordings"));
        assert_eq!(read(&elsewhere.join("a.wav")), b"a");
    }
}
