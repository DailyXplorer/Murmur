//! A recorded meeting seen from the library's side, on invented audio: it is
//! processed with Codex, the analysis and the summary scripted, then listed,
//! opened, found by a search, exported, set aside through the Trash and
//! deleted. Nothing here is specific to a platform, so it runs on the Windows
//! CI as on the Mac; what is specific (devices, WASAPI, the Recycle Bin call)
//! has its own tests next to its code.

use super::*;
use crate::meetings::trash;
use crate::meetings::trash::tests::FolderTrash;

#[test]
fn a_recorded_meeting_is_processed_listed_found_exported_and_deleted() {
    let h = harness();
    let id = spoken_meeting(&h);
    let summarizer = FakeSummarizer::answering(vec![Ok(
        "# Point budget\n\n[[s1]] et [[me]] valident le budget.".into(),
    )]);

    h.process_all(
        id,
        codex(|_| Ok("Le budget est validé.".into())),
        Arc::new(FakeAnalyzer::without_models()),
        summarizer,
    );

    // The list shows it, titled by its summary.
    let page = h.store.list(None, 10).unwrap();
    assert_eq!(page.meetings.len(), 1);
    assert_eq!(page.meetings[0].id, id);
    assert_eq!(page.meetings[0].title.as_deref(), Some("Point budget"));
    assert_eq!(page.meetings[0].status, MeetingStatus::Ready);

    // The detail has both voices and the summary.
    let meeting = h.store.get(id).unwrap().unwrap();
    assert!(!meeting.turns.is_empty());
    assert!(meeting.summary.is_some());

    // A word of the transcript and a word of the title both find it.
    for query in ["validé", "budget"] {
        let hits = h.store.search(query, 50).unwrap();
        assert_eq!(hits.len(), 1, "{query}");
        assert_eq!(hits[0].id, id, "{query}");
    }
    assert!(h.store.search("inconnu", 50).unwrap().is_empty());

    // The notes the Trash receives name the speakers, not their keys.
    let notes = trash::Notes::of(&meeting).unwrap();
    assert!(notes.file_name.ends_with("Point budget.md"));
    assert!(notes.markdown.contains("valident le budget"));
    assert!(notes.markdown.contains("Le budget est validé."));
    assert!(!notes.markdown.contains("[["));

    // Delete: the folder goes to the Trash first, the rows after.
    let bin = FolderTrash::new(&h._dir.path().join("trash"));
    trash::trash_meeting(&h.root, id, &notes, &bin).unwrap();
    let stem = notes.file_name.trim_end_matches(".md");
    assert!(bin.bin.join(stem).join(&notes.file_name).is_file());
    assert!(!paths::meeting_dir(&h.root, id).exists());
    h.store.delete(id).unwrap();

    assert!(h.store.list(None, 10).unwrap().meetings.is_empty());
    assert!(h.store.search("budget", 50).unwrap().is_empty());
    assert!(h.store.get(id).unwrap().is_none());
}
