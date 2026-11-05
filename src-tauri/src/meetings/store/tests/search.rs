use super::*;

#[test]
fn search_finds_accented_word_without_accent() {
    let (_dir, store) = store();
    let id = meeting(
        &store,
        1_000,
        &["Bonjour à tous.", "La réunion de l'été commence."],
        Some(("Réunion d'équipe", "Point sur l'été.")),
    );
    meeting(&store, 2_000, &["Nothing related."], None);

    let hits = store.search("reunion ete", 50).unwrap();

    assert_eq!(hits.len(), 1);
    let hit = &hits[0];
    assert_eq!(hit.id, id);
    assert_eq!(hit.first_turn_seq, Some(1));
    assert_eq!(marked(hit.title.as_ref().unwrap()), ["Réunion"]);
    assert_eq!(hit.snippets.len(), 2);
    assert_eq!(hit.snippets[0].text, "Point sur l'été.");
    assert_eq!(marked(&hit.snippets[1]), ["réunion", "été"]);
}

#[test]
fn search_prefix_matches() {
    let (_dir, store) = store();
    let id = meeting(&store, 1_000, &["We reviewed the pricing model."], None);

    assert_eq!(found(&store, "pric"), [id]);
    assert_eq!(found(&store, "PRICING mod"), [id]);
    assert_eq!(found(&store, "\"pricing model\""), [id]);
    assert!(found(&store, "pricey").is_empty());
    assert!(found(&store, "\"model pricing\"").is_empty());
    assert!(found(&store, "p").is_empty());
}

#[test]
fn search_matches_words_spread_across_a_meeting() {
    let (_dir, store) = store();
    let id = meeting(
        &store,
        1_000,
        &["We talked about hiring.", "Then the roadmap slipped."],
        Some(("Budget review", "Numbers.")),
    );
    let roadmap_only = meeting(&store, 2_000, &["The roadmap again."], None);

    assert_eq!(found(&store, "budget roadmap"), [id]);
    assert_eq!(found(&store, "hiring roadmap"), [id]);
    assert_eq!(found(&store, "roadmap"), [roadmap_only, id]);
    assert!(found(&store, "budget forecast").is_empty());
    let hits = store.search("hiring roadmap", 50).unwrap();
    assert_eq!(hits[0].first_turn_seq, Some(0));
    let marks: Vec<Vec<String>> = hits[0].snippets.iter().map(marked).collect();
    assert_eq!(marks, [["hiring"], ["roadmap"]]);
}

#[test]
fn search_ranks_title_hit_first() {
    let (_dir, store) = store();
    let transcript = meeting(
        &store,
        3_000,
        &["The roadmap, the roadmap, and the roadmap again."],
        Some(("Weekly sync", "Hiring.")),
    );
    let title = meeting(
        &store,
        1_000,
        &["Hello."],
        Some(("Roadmap review", "Hiring.")),
    );
    let summary = meeting(
        &store,
        2_000,
        &["Hello."],
        Some(("Planning", "The roadmap slips by [[me]].")),
    );

    assert_eq!(found(&store, "roadmap"), [title, summary, transcript]);
    let hits = store.search("roadmap", 50).unwrap();
    assert_eq!(hits[1].snippets[0].text, "The roadmap slips by Me.");
}

#[test]
fn deleted_meeting_leaves_no_fts_rows() {
    let (_dir, store) = store();
    let id = meeting(
        &store,
        1_000,
        &["Budget first.", "Budget again."],
        Some(("Budget", "Budget.")),
    );
    let conn = store.connection().unwrap();
    assert_eq!(indexed_rows(&conn), (1, 2));

    store.delete(id).unwrap();

    assert_eq!(indexed_rows(&conn), (0, 0));
    assert!(found(&store, "budget").is_empty());
}

#[test]
fn reprocessed_turns_are_reindexed() {
    let (_dir, store) = store();
    let id = meeting(
        &store,
        1_000,
        &["The draft wording."],
        Some(("Draft", "About the draft.")),
    );
    assert_eq!(found(&store, "draft"), [id]);

    assert!(store.retranscribe(id).unwrap());
    assert!(store.begin_processing(id).unwrap());
    store
        .save_transcript(
            id,
            &[me()],
            &said(&["The final wording."]),
            SpeakerAnalysis::NoModels,
            None,
            None,
        )
        .unwrap();

    // The new words replace the old, and the summary that quoted them
    // is gone until it is written again; the title stays.
    let hits = store.search("draft", 50).unwrap();
    assert_eq!(hits.len(), 1);
    assert!(hits[0].snippets.is_empty());
    assert_eq!(hits[0].first_turn_seq, None);
    assert_eq!(found(&store, "final"), [id]);
    let conn = store.connection().unwrap();
    assert_eq!(indexed_rows(&conn), (1, 1));
    conn.execute(
        "INSERT INTO meeting_turns_fts(meeting_turns_fts) VALUES ('integrity-check')",
        [],
    )
    .unwrap();
}

/// Pseudo-random words with a Zipf-like spread, as speech has: a few
/// words are everywhere, most are rare.
struct Words {
    vocabulary: Vec<String>,
    cumulative: Vec<f64>,
    state: u64,
}

impl Words {
    fn new() -> Self {
        const SYLLABLES: [&str; 24] = [
            "ba", "ko", "ri", "ten", "mo", "la", "vi", "sun", "de", "pra", "lo", "fe", "ga", "ni",
            "tor", "pe", "su", "mar", "di", "ca", "ble", "ro", "ve", "nu",
        ];
        let vocabulary: Vec<String> = (0..20_000usize)
            .map(|index| {
                let mut word = String::new();
                let mut rest = index;
                loop {
                    word.push_str(SYLLABLES[rest % SYLLABLES.len()]);
                    rest /= SYLLABLES.len();
                    if rest == 0 {
                        break;
                    }
                }
                word
            })
            .collect();
        let mut total = 0.0;
        let cumulative = (1..=vocabulary.len())
            .map(|rank| {
                total += 1.0 / rank as f64;
                total
            })
            .collect();
        Self {
            vocabulary,
            cumulative,
            state: 0x9E37_79B9_7F4A_7C15,
        }
    }

    fn next_unit(&mut self) -> f64 {
        self.state ^= self.state << 13;
        self.state ^= self.state >> 7;
        self.state ^= self.state << 17;
        (self.state >> 11) as f64 / (1u64 << 53) as f64
    }

    fn word(&mut self) -> &str {
        let target = self.next_unit() * self.cumulative.last().unwrap();
        let rank = self.cumulative.partition_point(|sum| *sum < target);
        &self.vocabulary[rank.min(self.vocabulary.len() - 1)]
    }

    fn sentence(&mut self, words: usize) -> String {
        (0..words)
            .map(|_| self.word().to_string())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

#[test]
#[ignore = "benchmark: cargo test --release search_bench -- --ignored --nocapture"]
fn search_bench() {
    const MEETINGS: i64 = 300;
    // One hour at 150 words a minute, in turns of 20 words.
    const TURNS: i64 = 450;
    const TURN_WORDS: usize = 20;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("meetings.db");
    let mut conn = Connection::open(&path).unwrap();
    migrations().to_version(&mut conn, 13).unwrap();
    let mut words = Words::new();
    let mut text_bytes = 0usize;
    let tx = conn.transaction().unwrap();
    for id in 1..=MEETINGS {
        let title = words.sentence(4);
        let summary = words.sentence(120);
        text_bytes += title.len() + summary.len();
        tx.execute(
            "INSERT INTO meetings
                 (id, started_at_ms, duration_ms, capture_mode, status, title, summary_md)
             VALUES (?1, ?2, 3600000, 'both', 'ready', ?3, ?4)",
            params![id, id * 86_400_000, title, summary],
        )
        .unwrap();
        for seq in 0..TURNS {
            let text = words.sentence(TURN_WORDS);
            text_bytes += text.len();
            tx.execute(
                "INSERT INTO meeting_turns (meeting_id, seq, speaker_key, start_ms, end_ms, text)
                 VALUES (?1, ?2, 'me', ?3, ?3 + 8000, ?4)",
                params![id, seq, seq * 8_000, text],
            )
            .unwrap();
        }
    }
    tx.commit().unwrap();
    let before = std::fs::metadata(&path).unwrap().len();

    let started = std::time::Instant::now();
    migrations().to_latest(&mut conn).unwrap();
    let migration = started.elapsed();
    drop(conn);
    let after = std::fs::metadata(&path).unwrap().len();

    let mut queries: Vec<String> = Vec::new();
    for index in 0..100 {
        let word = words.word().to_string();
        queries.push(match index % 5 {
            0 => word[..2].to_string(),
            1 => word[..3.min(word.len())].to_string(),
            2 => word,
            3 => format!("{word} {}", words.word()),
            _ => format!("\"{word} {}\"", words.word()),
        });
    }
    let (store, _) = MeetingsStore::open(path, now());
    const KINDS: [&str; 5] = [
        "2-letter prefix",
        "3-letter prefix",
        "one word",
        "two words",
        "phrase",
    ];
    let mut times = Vec::with_capacity(queries.len());
    let mut by_kind = vec![Vec::new(); KINDS.len()];
    let mut hits = 0;
    for (index, query) in queries.iter().enumerate() {
        let started = std::time::Instant::now();
        hits += store
            .search(query, crate::search::SEARCH_LIMIT)
            .unwrap()
            .len();
        times.push(started.elapsed());
        by_kind[index % KINDS.len()].push(started.elapsed());
    }
    times.sort();
    for (kind, mut times) in KINDS.iter().zip(by_kind) {
        times.sort();
        println!(
            "search_bench: {kind}: p50 {:.2?}, max {:.2?}",
            times[times.len() / 2],
            times[times.len() - 1]
        );
    }
    println!(
        "search_bench: {MEETINGS} meetings x {TURNS} turns, {:.1} MB of text",
        text_bytes as f64 / 1e6
    );
    println!(
        "search_bench: db {:.1} MB before, {:.1} MB after the FTS migration (+{:.0}%), migration {:.2?}",
        before as f64 / 1e6,
        after as f64 / 1e6,
        (after as f64 / before as f64 - 1.0) * 100.0,
        migration
    );
    println!(
        "search_bench: {} queries, p50 {:.2?}, p95 {:.2?}, max {:.2?}, {:.1} hits per query",
        times.len(),
        times[times.len() / 2],
        times[times.len() * 95 / 100],
        times[times.len() - 1],
        hits as f64 / times.len() as f64
    );
}
