use super::*;

#[test]
fn names_inside_sentences_are_found_and_sentence_starts_are_not() {
    let text =
        "Thanks for the update. I spoke with Ingrid and Tobias yesterday.\nShipping is on track.";
    assert_eq!(candidate_names(text), vec!["Ingrid", "Tobias"]);
}

#[test]
fn interface_labels_at_line_start_are_skipped() {
    assert!(candidate_names("File\nEdit\nView\nSearch messages").is_empty());
}

#[test]
fn capitalised_common_words_seen_in_lowercase_are_skipped() {
    let text = "Open the Settings page. the settings are saved with Murmur.";
    assert_eq!(candidate_names(text), vec!["Murmur"]);
}

#[test]
fn camel_case_brands_count_even_at_sentence_start() {
    assert_eq!(
        candidate_names("GitHub and iPhone users. macOS ships soon."),
        vec!["GitHub", "iPhone", "macOS"]
    );
}

#[test]
fn emails_and_handles_give_their_name_parts() {
    assert_eq!(
        candidate_names("reply to jane.doe@example.com or ping @kai_dev today"),
        vec!["Jane", "Doe", "kai_dev"]
    );
}

#[test]
fn acronyms_numbers_and_urls_are_not_names() {
    assert!(candidate_names("see the PDF and 2024 and www.Example.com now").is_empty());
}

#[test]
fn names_are_deduplicated_ranked_by_frequency_and_capped() {
    let text = "meet Zed then Amaya then Zed again";
    assert_eq!(candidate_names(text), vec!["Zed", "Amaya"]);

    let many: String = (0..80).map(|i| format!("with Name{i} ")).collect();
    assert_eq!(candidate_names(&many).len(), MAX_NAMES);
}

#[test]
fn generated_tokens_are_not_names() {
    for token in [
        // Generated passwords.
        "Xk9mPq2vLr7T",
        "Tr0ub4dor&3",
        "hGqWzKpLxV",
        "Xkqzvbnm",
        // Base62, base64 and hex tokens.
        "aB3dE5fG7hJ9kL1mN0pQ",
        "Z7kQ2mN9xR4bL1vT8cW3",
        "dGhpcyBpcyBhIHNlY3JldA==",
        "Deadbeef0042",
        "Fa3c9e1b7d5a",
        // AWS-like access key id and secret key.
        "AKIAIOSFODNN7EXAMPLE",
        "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
    ] {
        assert!(
            candidate_names(&format!("see {token} here")).is_empty(),
            "accepted {token}"
        );
    }
}

#[test]
fn real_names_and_brands_are_still_names() {
    let text = "met McDonald, O'Brien, Jean-Luc, Schwarzschild and Strzelczyk about iPhone15, \
         GPT-4o, OpenAI and DeepSeek";
    assert_eq!(
        candidate_names(text),
        vec![
            "McDonald",
            "O'Brien",
            "Jean-Luc",
            "Schwarzschild",
            "Strzelczyk",
            "iPhone15",
            "GPT-4o",
            "OpenAI",
            "DeepSeek"
        ]
    );
    for word in [
        "GPT-4",
        "M3",
        "iPhone",
        "Pixel8a",
        "Hewlett-Packard-Enterprise",
    ] {
        assert!(!looks_generated(word), "rejected {word}");
    }
}

/// `PENDING` is shared by every dictation, so its tests take turns.
static PENDING_TESTS: Mutex<()> = Mutex::new(());

fn names(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| name.to_string()).collect()
}

fn field(before: &str, after: &str) -> Option<FieldExcerpt> {
    Some(FieldExcerpt {
        before: before.to_string(),
        after: after.to_string(),
    })
}

#[test]
fn the_kind_of_app_is_captured_at_the_start_even_with_names_and_field_off() {
    let _turn = PENDING_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let only_kind = ContextReads {
        kind: true,
        ..ContextReads::default()
    };
    capture_with(only_kind, |reads| WindowContext {
        kind: reads.kind.then_some(AppCategory::Emails),
        ..WindowContext::default()
    });
    assert_eq!(collect().kind, Some(AppCategory::Emails));
}

#[test]
fn a_dictation_started_with_names_off_drops_an_unclaimed_capture() {
    let _turn = PENDING_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let (sender, receiver) = mpsc::channel();
    sender
        .send(WindowContext {
            kind: None,
            names: names(&["Ingrid"]),
            field: field("Hi Ingrid, ", ""),
        })
        .unwrap();
    *PENDING.lock().unwrap() = Some(receiver);
    capture(ContextReads::default());
    assert_eq!(collect(), WindowContext::default());
}

#[test]
fn each_read_runs_only_when_asked_for() {
    use std::sync::Arc;

    let _turn = PENDING_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let excerpt = || field("Hi Priya, ", "");
    for (reads, expected_reads, expected) in [
        (ContextReads::default(), vec![], WindowContext::default()),
        (
            ContextReads {
                kind: false,
                names: true,
                field: false,
            },
            vec!["window"],
            WindowContext {
                kind: None,
                names: names(&["Ingrid"]),
                field: None,
            },
        ),
        (
            ContextReads {
                kind: false,
                names: false,
                field: true,
            },
            vec!["field"],
            WindowContext {
                kind: None,
                names: Vec::new(),
                field: excerpt(),
            },
        ),
        (
            ContextReads {
                kind: false,
                names: true,
                field: true,
            },
            vec!["field", "window"],
            WindowContext {
                kind: None,
                names: names(&["Priya", "Ingrid"]),
                field: excerpt(),
            },
        ),
    ] {
        let asked = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&asked);
        capture_with(reads, move |reads| {
            let field_log = Arc::clone(&log);
            let context = read_window(
                reads,
                move || {
                    field_log.lock().unwrap().push("field");
                    excerpt()
                },
                || {
                    log.lock().unwrap().push("window");
                    "meet Ingrid today".to_string()
                },
            );
            log.lock().unwrap().push("done");
            context
        });
        let started = Instant::now();
        while !asked.lock().unwrap().contains(&"done")
            && started.elapsed() < Duration::from_secs(10)
        {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(collect(), expected, "{reads:?}");
        let asked: Vec<_> = asked
            .lock()
            .unwrap()
            .iter()
            .copied()
            .filter(|read| *read != "done")
            .collect();
        assert_eq!(asked, expected_reads, "{reads:?}");
    }
}

#[test]
fn the_excerpt_and_names_are_sent_as_read() {
    let all = ContextReads {
        kind: false,
        names: true,
        field: true,
    };
    let context = read_window(
        all,
        || field("Code porte : 4512B\n", ""),
        || "Wi-Fi password: Bluebird\nshared with Corvina".to_string(),
    );
    assert_eq!(
        context,
        WindowContext {
            kind: None,
            names: names(&["Wi-Fi", "Corvina"]),
            field: field("Code porte : 4512B\n", ""),
        }
    );
}

/// Every text the name finder was checked on, and the names it gives.
/// Add rows; never change one to make a fix pass.
const NAMES_CORPUS: &[(&str, &[&str])] = &[
    (
        "Thanks for the update. I spoke with Ingrid and Tobias yesterday.\nShipping is on track.",
        &["Ingrid", "Tobias"],
    ),
    ("File\nEdit\nView\nSearch messages", &[]),
    (
        "Open the Settings page. the settings are saved with Murmur.",
        &["Murmur"],
    ),
    (
        "GitHub and iPhone users. macOS ships soon.",
        &["GitHub", "iPhone", "macOS"],
    ),
    (
        "reply to jane.doe@example.com or ping @kai_dev today",
        &["Jane", "Doe", "kai_dev"],
    ),
    ("see the PDF and 2024 and www.Example.com now", &[]),
    ("meet Zed then Amaya then Zed again", &["Zed", "Amaya"]),
    ("see Xk9mPq2vLr7T here", &[]),
    ("see Tr0ub4dor&3 here", &[]),
    ("see hGqWzKpLxV here", &[]),
    ("see Xkqzvbnm here", &[]),
    ("see aB3dE5fG7hJ9kL1mN0pQ here", &[]),
    ("see Z7kQ2mN9xR4bL1vT8cW3 here", &[]),
    ("see dGhpcyBpcyBhIHNlY3JldA== here", &[]),
    ("see Deadbeef0042 here", &[]),
    ("see Fa3c9e1b7d5a here", &[]),
    ("see AKIAIOSFODNN7EXAMPLE here", &[]),
    ("see wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY here", &[]),
    (
        "met McDonald, O'Brien, Jean-Luc, Schwarzschild and Strzelczyk about iPhone15, \
         GPT-4o, OpenAI and DeepSeek",
        &[
            "McDonald",
            "O'Brien",
            "Jean-Luc",
            "Schwarzschild",
            "Strzelczyk",
            "iPhone15",
            "GPT-4o",
            "OpenAI",
            "DeepSeek",
        ],
    ),
];

#[test]
fn names_corpus() {
    let wrong: Vec<String> = NAMES_CORPUS
        .iter()
        .filter(|(text, expected)| candidate_names(text) != names(expected))
        .map(|(text, expected)| format!("{text:?}: {:?}, not {expected:?}", candidate_names(text)))
        .collect();
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// Measures every read on each regular app running, without bringing
/// any to the front, and writes a markdown table. Only lengths, counts,
/// roles and attribute names are recorded, never text.
#[cfg(target_os = "macos")]
#[test]
#[ignore]
fn coverage_of_running_apps() {
    use crate::accessibility::{is_password_app, is_trusted};
    use crate::field_excerpt::{MAX_AFTER_CHARS, MAX_BEFORE_CHARS};
    use objc2_app_kit::{NSApplicationActivationPolicy, NSWorkspace};
    use std::fmt::Write as _;

    let out = std::env::var("MURMUR_COVERAGE_OUT")
        .unwrap_or_else(|_| "/tmp/murmur-i153-coverage.md".to_string());
    let chars = |text: Option<String>| {
        text.map_or("-".to_string(), |text| text.chars().count().to_string())
    };
    let mut main = format!(
        "# Window context coverage\n\n{} UTC, Accessibility trusted: {}\n\n\
         Before: the window read at 71afa240 (window chars, names, ms). After: the \
         production reader with names and field on (excerpt chars before/after the \
         cursor, names, ms).\n\n\
         | App | Before | After | Notes |\n|---|---|---|---|\n",
        chrono::Utc::now().format("%Y-%m-%d %H:%M"),
        is_trusted()
    );
    let mut report = String::from(
        "\n## Probe details\n\n\
         (a) selection + AXStringForRange of the reader's window, (b) selection + AXValue, \
         (c) AXSelectedText, (d) AXVisibleCharacterRange length, (e) BFS text of the nearest \
         AXWebArea above the focus. Lengths in chars; `-` is a failed read.\n\n\
         | app | focus role/subrole | secure | selection | (a) | (b) | (c) | (d) | (e) chars, ms \
         | before: window chars, names, ms | after: excerpt chars, names, ms |\n\
         |---|---|---|---|---|---|---|---|---|---|---|\n",
    );
    let mut attributes = String::from("\n## Attributes of the focused element\n");
    let workspace = NSWorkspace::sharedWorkspace();
    for running in workspace.runningApplications().iter() {
        let pid = running.processIdentifier();
        let bundle_id = running
            .bundleIdentifier()
            .map(|id| id.to_string())
            .unwrap_or_default();
        if running.activationPolicy() != NSApplicationActivationPolicy::Regular
            || pid <= 0
            || bundle_id.to_lowercase().contains("murmur")
            || is_password_app(&bundle_id)
        {
            continue;
        }
        let Some(app) = Element::application(pid) else {
            continue;
        };
        let focused = app.element("AXFocusedUIElement");
        let role = focused.as_ref().map_or("no focus".to_string(), |focused| {
            format!(
                "{}/{}",
                focused.string("AXRole").unwrap_or_default(),
                focused.string("AXSubrole").unwrap_or_default()
            )
        });
        let secure = focused.as_ref().is_some_and(Element::is_secure);
        let readable = focused.as_ref().filter(|_| !secure);
        let selection = readable.and_then(|focused| focused.range("AXSelectedTextRange"));
        let ranged = readable.zip(selection).and_then(|(focused, selection)| {
            let count = FocusedText::char_count(focused)?;
            let end = (selection.start + selection.len + 2 * MAX_AFTER_CHARS + 2).min(count);
            let start = selection
                .start
                .saturating_sub(2 * MAX_BEFORE_CHARS + 2)
                .min(end);
            focused.string_for_range(Utf16Range {
                start,
                len: end - start,
            })
        });
        let value = readable
            .filter(|_| selection.is_some())
            .and_then(|focused| focused.string("AXValue"));
        let selected = readable.and_then(|focused| focused.string("AXSelectedText"));
        let visible = readable
            .and_then(|focused| focused.range("AXVisibleCharacterRange"))
            .map(|range| range.len.to_string());
        let web_area = readable.and_then(|focused| {
            let mut element = focused.clone();
            for _ in 0..30 {
                if element.string("AXRole").as_deref() == Some("AXWebArea") {
                    return Some(element);
                }
                element = element.element("AXParent")?;
            }
            None
        });
        let web_area = web_area.map_or("-".to_string(), |web_area| {
            let started = Instant::now();
            let text = window_text(web_area, started);
            format!(
                "{}, {}",
                text.chars().count(),
                started.elapsed().as_millis()
            )
        });

        let started = Instant::now();
        let window = app
            .element("AXFocusedWindow")
            .map(|window| window_text(window, started));
        let before = window.map_or("no window".to_string(), |text| {
            let elapsed = started.elapsed().as_millis();
            let names = names_in(None, &text).len();
            format!("{}, {names}, {elapsed}", text.chars().count())
        });

        let started = Instant::now();
        let all = ContextReads {
            kind: false,
            names: true,
            field: true,
        };
        let context = read_app(all, pid, &app, started);
        let elapsed = started.elapsed().as_millis();
        let excerpt = context.field.as_ref().map_or(0, |field| {
            field.before.chars().count() + field.after.chars().count()
        });
        let after = format!("{excerpt}, {}, {elapsed}", context.names.len());
        let split = context.field.as_ref().map_or("0/0".to_string(), |field| {
            format!(
                "{}/{}",
                field.before.chars().count(),
                field.after.chars().count()
            )
        });
        let worked: Vec<&str> = [
            ("selection", selection.is_some()),
            ("AXStringForRange", ranged.is_some()),
            ("AXValue", value.is_some()),
            ("AXSelectedText", selected.is_some()),
            ("AXVisibleCharacterRange", visible.is_some()),
            ("web area", web_area != "-"),
        ]
        .into_iter()
        .filter_map(|(read, ok)| ok.then_some(read))
        .collect();
        let app_name = running
            .localizedName()
            .map_or_else(|| bundle_id.clone(), |name| name.to_string());
        writeln!(
            main,
            "| {app_name} | {before} | {split}, {}, {elapsed} | {role}{}; worked: {} |",
            context.names.len(),
            if secure { ", secure" } else { "" },
            if worked.is_empty() {
                "none".to_string()
            } else {
                worked.join(", ")
            }
        )
        .unwrap();

        writeln!(
            report,
            "| {bundle_id} | {role} | {secure} | {} | {} | {} | {} | {} | {web_area} | {before} | {after} |",
            selection.map_or("-".to_string(), |selection| format!("len {}", selection.len)),
            chars(ranged),
            chars(value),
            chars(selected),
            visible.unwrap_or_else(|| "-".to_string()),
        )
        .unwrap();
        if let Some(focused) = &focused {
            writeln!(
                attributes,
                "\n- {bundle_id}: {}; parameterized: {}",
                focused.attribute_names(false).join(", "),
                focused.attribute_names(true).join(", ")
            )
            .unwrap();
        }
    }
    main.push_str(&report);
    main.push_str(&attributes);
    std::fs::write(&out, &main).unwrap();
    println!("{main}");
}

#[test]
fn names_in_the_excerpt_come_before_the_window_names() {
    let excerpt = field("Dear Siobhán and Priya, ", "with Ingrid");
    assert_eq!(
        names_in(excerpt.as_ref(), "meet Tobias and Siobhán today"),
        names(&["Siobhán", "Priya", "Ingrid", "Tobias"])
    );
}

/// A window's tree.
#[derive(Clone, Default)]
struct Node {
    text: &'static str,
    secure: bool,
    children: Vec<Node>,
}

impl WindowNode for Node {
    fn is_secure(&self) -> bool {
        self.secure
    }

    fn texts(&self) -> Vec<String> {
        vec![self.text.to_string()]
    }

    fn children(&self) -> Vec<Self> {
        self.children.clone()
    }
}

fn node(text: &'static str, children: Vec<Node>) -> Node {
    Node {
        text,
        children,
        ..Node::default()
    }
}

#[test]
fn the_window_is_read_breadth_first_without_secure_fields() {
    let window = node(
        "Draft",
        vec![
            node("To", vec![node("Ingrid", vec![])]),
            Node {
                secure: true,
                ..node("hunter2", vec![node("inside the password field", vec![])])
            },
            node("Subject", vec![]),
        ],
    );
    assert_eq!(
        window_text(window, Instant::now()),
        "Draft\nTo\nSubject\nIngrid\n"
    );
}

#[test]
fn the_window_read_stops_after_the_node_limit() {
    let window = node("Window", vec![node("Item", vec![]); 2 * MAX_NODES]);
    let text = window_text(window, Instant::now());
    assert_eq!(text.lines().count(), MAX_NODES);
}

/// The budget is a question to the clock, so a step clock that says
/// "spent" at its ninth look stands for a slow app without sleeping.
#[test]
fn the_window_read_stops_when_the_budget_is_spent() {
    let window = node("Window", vec![node("Item", vec![]); 40]);
    let mut looks = 0;
    let text = window_text_until(window, || {
        looks += 1;
        looks > 8
    });
    assert_eq!(text.lines().count(), 8, "{text:?}");
    assert_eq!(looks, 9, "the read looks once per element, then stops");
}

#[test]
fn a_read_budget_spent_before_the_start_reads_nothing() {
    let window = node("Window", vec![node("Item", vec![]); 4]);
    let long_ago = Instant::now()
        .checked_sub(READ_BUDGET + Duration::from_secs(1))
        .expect("a clock that has run for more than a second");
    assert_eq!(window_text(window, long_ago), "");
}

/// The production reads on real Win32 controls, in windows this test
/// opens, so no foreground window is needed.
#[cfg(target_os = "windows")]
mod uia {
    use super::*;
    use crate::field_excerpt::{cut, read_around_selection, Utf16Range};
    use crate::uia::test_fields::{self, requests, Read, PLAIN_TEXT, SECRET_TEXT};
    use crate::uia::test_window::{child, TestWindow, PLAIN};
    use crate::uia::{Automation, Element};
    use windows::core::w;
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::System::LibraryLoader::LoadLibraryW;
    use windows::Win32::UI::WindowsAndMessaging::{
        SendMessageW, ES_AUTOHSCROLL, ES_PASSWORD, WINDOW_STYLE, WS_BORDER,
    };

    const EM_SETSEL: u32 = 0x00B1;
    /// The rich edit control of Msftedit.dll, as in WordPad and Windows
    /// 11's Notepad.
    const RICH_EDIT: &str = "RICHEDIT50W";

    fn later() -> Instant {
        Instant::now() + Duration::from_secs(60)
    }

    /// About 1 400 chars, so each side of the middle is longer than the
    /// excerpt.
    fn prose() -> String {
        "Dear Ingrid, the report from Tobias is ready for review. ".repeat(24)
    }

    /// A window holding one field of `class` with `text` and the
    /// selection `selection`, and a label.
    fn field_window(
        class: &'static str,
        style: WINDOW_STYLE,
        text: String,
        selection: Utf16Range,
    ) -> TestWindow {
        if class == RICH_EDIT {
            // SAFETY: loads a system library that registers the class.
            unsafe { LoadLibraryW(w!("Msftedit.dll")) }.expect("Msftedit.dll");
        }
        TestWindow::open(move |window| {
            let field = child(window, class, &text, WS_BORDER | style);
            // SAFETY: `field` is a live window of this thread.
            unsafe {
                SendMessageW(
                    field,
                    EM_SETSEL,
                    Some(WPARAM(selection.start)),
                    Some(LPARAM((selection.start + selection.len) as isize)),
                )
            };
            child(
                window,
                "STATIC",
                "Lunch with Amaya",
                WINDOW_STYLE::default(),
            );
            vec![field.0 as isize]
        })
    }

    fn element(automation: &Automation, window: isize) -> Element {
        automation.window(window).expect("element of the window")
    }

    #[test]
    fn the_excerpt_matches_the_one_cut_from_the_whole_field() {
        let automation = Automation::new().expect("UI Automation client");
        let text = prose();
        for class in ["EDIT", RICH_EDIT] {
            for selection in [
                Utf16Range { start: 700, len: 0 },
                Utf16Range {
                    start: 700,
                    len: 12,
                },
            ] {
                let window = field_window(
                    class,
                    WINDOW_STYLE(ES_AUTOHSCROLL as u32),
                    text.clone(),
                    selection,
                );
                let field = element(&automation, window.handles[0]);
                let excerpt = read_around_selection(&field, later());
                assert!(excerpt.is_some(), "{class} {selection:?}");
                assert_eq!(excerpt, cut(&text, selection), "{class} {selection:?}");
            }
        }
    }

    #[test]
    fn a_password_field_is_never_read() {
        let automation = Automation::new().expect("UI Automation client");
        let window = field_window(
            "EDIT",
            WINDOW_STYLE(ES_PASSWORD as u32),
            "hunter2 Corvina".to_string(),
            Utf16Range { start: 7, len: 0 },
        );
        let field = element(&automation, window.handles[0]);
        assert!(field.is_secure());
        assert_eq!(read_around_selection(&field, later()), None);
        let root = element(&automation, window.window);
        let text = window_text(
            UiaNode {
                automation: &automation,
                element: root,
            },
            Instant::now(),
        );
        assert!(text.contains("Amaya"), "the label is read");
        assert!(!text.contains("hunter2") && !text.contains("Corvina"));
    }

    #[test]
    fn a_password_fields_value_is_never_requested() {
        let automation = Automation::new().expect("UI Automation client");
        let window = test_fields::open();
        let secret = element(&automation, window.handles[0]);
        let secure = secret.is_secure();
        let value = secret.value();
        let excerpt = read_around_selection(&secret, later());
        let text = window_text(
            UiaNode {
                automation: &automation,
                element: element(&automation, window.window),
            },
            Instant::now(),
        );
        assert_eq!(
            [requests(true, Read::Value), requests(true, Read::Text)],
            [0, 0],
            "the password field's text"
        );
        assert!(secure);
        assert_eq!((value, excerpt), (None, None));
        assert!(!text.contains(SECRET_TEXT));
        assert!(text.contains(PLAIN_TEXT), "the other field is read");
        assert!(requests(false, Read::Value) > 0);
    }

    #[test]
    fn a_control_without_text_or_caret_gives_no_excerpt() {
        let automation = Automation::new().expect("UI Automation client");
        let window = TestWindow::open(|window| {
            vec![child(window, "BUTTON", "Send", WINDOW_STYLE::default()).0 as isize]
        });
        let button = element(&automation, window.handles[0]);
        assert_eq!(read_around_selection(&button, later()), None);
    }

    #[test]
    fn names_come_from_the_field_and_the_rest_of_the_window() {
        let automation = Automation::new().expect("UI Automation client");
        let window = field_window(
            RICH_EDIT,
            WINDOW_STYLE(ES_AUTOHSCROLL as u32),
            prose(),
            Utf16Range { start: 700, len: 0 },
        );
        let all = ContextReads {
            kind: false,
            names: true,
            field: true,
        };
        let started = Instant::now();
        let context = read_window(
            all,
            || {
                let field = element(&automation, window.handles[0]);
                read_around_selection(&field, started + FIELD_BUDGET)
            },
            || {
                let root = element(&automation, window.window);
                window_text(
                    UiaNode {
                        automation: &automation,
                        element: root,
                    },
                    started,
                )
            },
        );
        assert!(context.field.is_some());
        for name in ["Ingrid", "Tobias", "Amaya"] {
            assert!(context.names.contains(&name.to_string()), "{name}");
        }
    }

    #[test]
    fn a_window_of_many_controls_is_read_within_the_budget() {
        let automation = Automation::new().expect("UI Automation client");
        let window = TestWindow::open(|window| {
            for _ in 0..40 {
                let group: HWND = child(window, PLAIN, "", WINDOW_STYLE::default());
                for item in 0..40 {
                    child(
                        group,
                        "STATIC",
                        &format!("Item {item}"),
                        WINDOW_STYLE::default(),
                    );
                }
            }
            Vec::new()
        });
        let root = element(&automation, window.window);
        // The first call into the window loads its providers, which an
        // app the user works in has long done.
        automation.children(&root);
        let started = Instant::now();
        let text = window_text(
            UiaNode {
                automation: &automation,
                element: root,
            },
            started,
        );
        let elapsed = started.elapsed();
        let lines = text.lines().count();
        assert!(text.starts_with("Murmur UI Automation test\n"));
        // The tree holds 1 641 controls, 1 601 of them named.
        assert!(lines < MAX_NODES, "{lines} lines in {elapsed:?}");
        // One call may take its whole timeout after the last check.
        assert!(
            elapsed < READ_BUDGET + Duration::from_millis(300),
            "{lines} lines in {elapsed:?}"
        );
    }
}
