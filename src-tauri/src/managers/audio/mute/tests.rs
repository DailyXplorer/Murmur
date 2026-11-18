use super::*;
use std::time::Instant;

#[test]
#[ignore = "reads the output mute of the Mac that runs it, 5 times through Core Audio and osascript; run with --ignored --nocapture"]
fn core_audio_and_osascript_read_the_same_mute() {
    for round in 1..=5 {
        let started = Instant::now();
        let hal = core_audio::muted();
        let hal_took = started.elapsed();
        let started = Instant::now();
        let script = get_mute_with_osascript();
        let script_took = started.elapsed();
        println!(
            "round {round}: Core Audio {hal:?} in {hal_took:?}, osascript {script:?} in {script_took:?}"
        );

        assert!(
            hal.is_some(),
            "this output has no mute Core Audio can read: pick the built-in speakers"
        );
        assert_eq!(hal, script);
    }
}

#[test]
#[ignore = "mutes or unmutes the Mac that runs it for a moment, then puts the mute back; run with --ignored --nocapture"]
fn core_audio_flips_the_mute_osascript_reads() {
    let before = core_audio::muted()
        .expect("this output has no mute Core Audio can read: pick the built-in speakers");

    let flipped = core_audio::set_muted(!before);
    let after_flip = (core_audio::muted(), get_mute_with_osascript());
    let put_back = core_audio::set_muted(before);
    if !put_back {
        set_mute_with_osascript(before);
    }
    let after_restore = (core_audio::muted(), get_mute_with_osascript());
    println!("before {before}, after the flip {after_flip:?}, restored {after_restore:?}");

    assert!(flipped, "Core Audio could not flip the mute");
    assert_eq!(after_flip, (Some(!before), Some(!before)));
    assert!(put_back, "Core Audio could not put the mute back");
    assert_eq!(after_restore, (Some(before), Some(before)));
}
