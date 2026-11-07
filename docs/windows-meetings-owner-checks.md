# Meetings on Windows: checks on a real PC

What no CI job can prove, for Meetings on Windows (system audio, speakers, calls, audio file and export, live transcript, notices, MCP, enabling, QA). Meetings are on by default now, so every check runs on the installer the Windows CI builds. `docs/windows-meetings.md` holds the parity table and, for each row, the tests that run in CI.

"Headset" means a wired or Bluetooth headset, "speakers" the PC's own. Tick a line once, on a real machine, and write the Windows build next to it.

## First run, upgrade and uninstall

- [ ] Install over a beta that had Meetings hidden: the Meetings page, the tray item and the Advanced switch appear, the switch is on and the shortcut field shows Alt+Shift+M.
- [ ] Turn the Advanced switch off, quit, relaunch: Meetings stay off, and the page and the tray item are gone.
- [ ] First meeting on a fresh install: the speech and speaker models download with progress shown, the meeting records meanwhile, and the transcript waits for the models or goes through Codex.
- [ ] Uninstall, then reinstall: the library (`meetings.db`, in `%APPDATA%\com.dailyxplorer.murmur`) and the audio folders (in `%LOCALAPPDATA%\com.dailyxplorer.murmur`) are still there and listed. Upgrading over this build keeps every meeting and its transcript.
- [ ] Upgrade from a build that kept `models`, `meetings` and `recordings` in `%APPDATA%\com.dailyxplorer.murmur`: after the first launch the three folders are in `%LOCALAPPDATA%\com.dailyxplorer.murmur`, every meeting plays and exports, every dictation in History plays, and the model is not downloaded again.
- [ ] Same upgrade with Roaming redirected to another drive or a network share (the roaming-profile case): the first launch copies instead of renaming, Murmur works the whole time, and the old folders are gone after it.
- [ ] Same upgrade with a meeting WAV open in another program: Murmur still starts, uses the old folder, and finishes the move at the next launch once the file is closed.
- [ ] `murmur.exe --help`, `murmur.exe -f <wav> --json` and `murmur.exe mcp` typed in cmd, PowerShell and Windows Terminal print their output; closing the terminal never closes a Murmur started with `--start-hidden`; an assistant's `murmur.exe mcp` still answers over its pipes.
- [ ] SmartScreen and the antivirus raise no warning on the installer, on the first meeting, on the loopback start, on the model download or on the first model load.
- [ ] Installer size against the previous beta (`Murmur_*_x64-setup.exe`); no `onnxruntime.dll` in the install folder.

## System audio

Run `cargo test --lib wasapi_ -- --ignored --nocapture` from `src-tauri` first on each machine.

- [ ] Windows 11: the log says "System audio: process loopback", and a video's sound is on the system track.
- [ ] Windows 10 before build 20348: the log says "classic loopback" after "process loopback unavailable", and a video's sound is on the system track.
- [ ] Murmur's start and stop chimes are absent from the system track on Windows 11, and present but harmless on classic loopback (the documented limit).
- [ ] A Teams call, a Zoom call and a Google Meet call in a browser: the remote voices are on the system track and the local voice only on the microphone track.
- [ ] Wired headset: unplug it mid-recording, the track keeps going on the speakers within about 2 s with the gap kept as silence and no crash; plug it back in, the track follows it.
- [ ] Bluetooth headset: connect and disconnect mid-recording (stereo and hands-free profiles), the track follows and the sound is neither sped up nor slowed down.
- [ ] Windows plays call audio on the default communications device: set a different default communications device and note whether the call's sound reaches the system track.
- [ ] Microphone on "Default", a USB or Bluetooth headset picked in the call app (Teams, Zoom, then Meet in Chrome) while the PC's default input is another microphone: the microphone track holds your voice, and the log says the meeting microphone follows the call app's input.
- [ ] Switch the microphone inside the call app mid-call: the track follows within a few seconds, with the gap kept as silence.
- [ ] A microphone picked in Murmur's Settings while the call app uses another: the track stays on the Settings microphone; mute that microphone at the hardware switch for 10 s and it moves to the call app's input.
- [ ] Two identical headsets: the track records the Settings or default microphone instead of guessing.
- [ ] A clap played through the speakers appears at the same time in both tracks (within about 50 ms).
- [ ] The output route reads speakers for laptop speakers, a Bluetooth speaker and an HDMI monitor, and headphones for wired headphones and a Bluetooth headset (`output_route_facts` in the log or a debugger).
- [ ] A 30 minute recording with a video playing: no dropped-audio warning, and the system WAV is as long as the microphone WAV.
- [ ] On speakers, the microphone hears the speakers and the transcript does not repeat the remote voices under "Me"; on a headset every microphone word stays.

## Sleep, disk and permissions

- [ ] An idle PC does not sleep during a recording and sleeps normally after it (`powercfg /requests` lists the request while recording and not after).
- [ ] Close the lid or sleep the PC for a minute during a meeting: the notice names the gap and the meeting continues.
- [ ] Microphone access off for desktop apps in Windows Settings: starting a meeting says so in Windows words (no "System Settings", no "Mac"), and "Open Windows Settings" opens Privacy & security, Microphone.
- [ ] Less than 1.5 GB free on the drive: the warning says "this PC".

## Recording a real call (Teams, Zoom, Google Meet in Chrome and Edge)

- [ ] With a headset, record 10 minutes: both voices are in the transcript, "Me" and "Others" are the right way round and the timing matches what was said.
- [ ] Summary quality: title, summary and the people named read right on an English call and on a French call.
- [ ] The red pill shows the elapsed time while recording, and a dictation made during a meeting still works.

## Speakers and word times

- [ ] A call with you and two remote people: the transcript shows "You" and two remote speakers, each line the right person's. Renaming one speaker keeps its name through "Transcribe again".
- [ ] A call with 4 remote speakers, then 5 or more with crosstalk and a headset: count the speakers shown against the people who spoke, and look at the turn boundaries.
- [ ] A French or Spanish call: the words are timed and the transcript is right; note how well the voices are told apart.
- [ ] Two remote people talking over each other for several seconds: the lines near the overlap go to a plausible speaker and none is lost.
- [ ] A meeting held in a room (everyone on the microphone, nothing on system audio): the microphone track is the one told apart.
- [ ] An English meeting with 2 to 3 speakers, then a meeting with a single speaker: no invented speaker and no merged ones.
- [ ] The same recording analysed on a Mac with the same models: the number of speakers and the change points agree; a difference is a bug to report with both `analysis.json` files.
- [ ] A low-end CPU (4 cores or fewer, no AVX2 if you have one): minutes to analyse a 1 h and a 3 h meeting (`Meeting analysis: … in N s`, `Meeting N timings:`), the peak working set of `murmur.exe`, the meeting still reaches "ready", and the analysis ends before its time limit. Without AVX2 each track keeps one speaker, the log says `the speaker model cannot run` and the app does not crash.
- [ ] A dictation made while a meeting is analysed pastes at its usual speed (the analysis uses half of the cores). The memory is released afterwards.
- [ ] Quit Murmur while a meeting is analysed, relaunch: the analysis runs again and finishes. Delete the meeting during a long analysis: `murmur.exe` stops using the CPU within a few seconds.
- [ ] Decline the speaker model (delete the models, start a meeting, cancel the download): the meeting is transcribed with one speaker per track and nothing shows an error; download again and analyse again, the speakers appear.
- [ ] Meetings turned on long after the dictation model was downloaded: the next meeting start fetches the 33 MB speaker model once in the background, and the analysis of that meeting waits for it. Cut the network during that download: the meeting still gets its transcript and the next one resumes it.
- [ ] A models folder an earlier build filled with the Sortformer model: after the next fetch the `sortformer` folder is gone, the speech model is not downloaded again and a dictation still works.

## Call detection and the prompts

Run with `--debug`: the log lists the apps on the microphone, by file name or package family, whenever the list changes.

- [ ] New Teams (packaged, `MSTeams_…`), classic Teams (`Teams.exe`) and Teams in a browser: joining a call shows "Record this meeting?" after 3 s. Write down the exact name in the log.
- [ ] Zoom, Slack huddle, Discord voice channel, Webex (note its executable): each prompts. Leaving the call while a meeting records shows "Call ended." 3 s later.
- [ ] Google Meet in Chrome, Edge, Firefox and Brave: the tab in front prompts, and switching to another tab of the same window keeps the prompt and the call.
- [ ] Meet behind another tab from the start: the address bar read finds it within about 5 s. Note which browsers expose the address bar to UI Automation.
- [ ] Mute in Teams, Zoom and Meet: "Call ended." must not appear.
- [ ] No prompt for Murmur's own dictation, a voice recorder, the Windows Voice Recorder, or a browser tab that only uses the microphone (voice search, a mic test page). With a browser holding the microphone and no meeting page, the app stays quiet over a minute (CPU in Task Manager).
- [ ] "Not now" does not ask again for the same call and asks again after the call left the microphone for 30 s and a new one starts. "Keep recording" asks again only after a new call (3 s on the microphone).
- [ ] Kill Zoom or Teams from Task Manager during a call: no prompt stays stuck and no "Call ended." is missing. For packaged new Teams a killed app may keep a stale entry: write down what happens.
- [ ] The prompt sits beside the orb on the monitor under the pointer, with the taskbar at the top, at 125 % and 150 % scaling, with the overlay at Top and at Bottom; it never takes the focus and is clickable. It is gone after each answer (no extra WebView2 process while idle).
- [ ] "Call ended." appears with Stop and transcribe and Keep recording; Keep recording asks again only after a call returns and leaves.
- [ ] Settings, Advanced, "Offer to record calls": no FaceTime in its description; turning it off hides a showing prompt and stops detection.

## Notices

- [ ] Finish a meeting: the orb opens "Transcript ready" with the title, Open and Copy, and it goes after 15 s. Two meetings close together read "2 meetings ready" with no Copy; a failure shows the failure notice with Open only.
- [ ] Placement on the primary and a secondary monitor, with the taskbar at the bottom, top, left and right, at 100 %, 150 % and 200 % scaling.
- [ ] With a text field focused in another app, clicking Open or Copy never moves the caret or the focus out of that app (Open then brings Settings forward on purpose). Hover keeps the notice up; Escape does nothing.
- [ ] Open lands on the right meeting, also when Settings was closed or minimized. Copy puts the Markdown in Notepad and the button reads "Copied" for a moment.
- [ ] A meeting open and visible in Settings gets no notice for itself; minimize Settings and a later finish does notify. A dictation replaces a showing notice, and a meeting that finished during a dictation shows its notice afterwards.

## Audio file, export and library

- [ ] A processed meeting ends with `audio.m4a` and no WAV tracks, about 43 MB an hour (a 10 minute meeting about 7 MB). It plays and seeks in the Meeting detail, Windows Media Player, Films and TV and a browser; the microphone is on the left channel, system audio on the right.
- [ ] Transcribe again from the compressed audio: the transcript comes back and word times line up within about 50 ms. A microphone-only and a system-only meeting each give a mono `audio.m4a` and transcribe again.
- [ ] "Delete audio" and the retention setting remove `audio.m4a`; deleting the audio while a long meeting is still compressing works or fails with a retryable error, and leaves no `audio.m4a.part`.
- [ ] An N or KN edition without the Media Feature Pack, and Windows Server without Media Foundation: Murmur starts normally, the meeting keeps its WAV tracks, `murmur.log` says "Media Foundation is not installed" or "AAC encoder failed", and after 3 launches the meeting is no longer retried.
- [ ] A 3 hour meeting compresses in a reasonable time and uses no more than a few hundred MB while it does.
- [ ] Export opens the Windows Save dialog over Murmur with the dated file name and `.md` added; it remembers the last folder, asks before replacing a file and Cancel writes nothing. Export into a very long path, a OneDrive-redirected Documents folder and a network share.
- [ ] Copy as Markdown pastes into Notepad with the speaker names. Search finds a word of the transcript and of the title, and opening a hit scrolls to the match.
- [ ] Delete a meeting: it is in the Recycle Bin as a folder named like the export holding the notes and the audio, and Murmur no longer lists it. "Delete audio" sends only the audio, in `<name> audio`, and the transcript stays.
- [ ] With "Don't move files to the Recycle Bin" on for the drive, or a Recycle Bin set very small, delete a meeting: Windows asks first. No keeps the meeting listed with its folder back; Yes erases it. Deleting a meeting that is recording is refused. Audio retention and Discard erase for good by design.

## Live transcript

- [ ] Record a 10 minute Teams call on a laptop CPU: the live text appears about 6 to 10 s after a sentence starts, "You" for the microphone and "Others" for the call; "Catching up" appears when the PC is slower than speech and goes away when people pause. On 4 cores or fewer the text may fall behind up to 120 s and skip pieces with a gap marker; the final transcript still has the skipped audio.
- [ ] Dictate during a live meeting (5 s): the text is pasted without a delay beyond the piece that was running. Time a dictation with live pieces running and with the live switch off; the priority only lowers the thread that calls the speech library, so this tells whether more is needed.
- [ ] Stop the meeting: the live draft is replaced by the final transcript and `live.ndjson` is gone. Kill Murmur during a meeting and relaunch: the draft shows as stopped until the final transcript is made.
- [ ] "Paste new meeting text" (bind keys on the Meetings page): paste into Word, Notepad and a Teams chat; lines read `You:` and `Others:`; a second press with nothing new says "Nothing new yet." It does nothing while a dictation records and does not take keys another action uses.
- [ ] On speakers a microphone sentence that is only the call's echo is hidden; on a headset every microphone sentence stays.
- [ ] Switch off in Advanced: the next meeting says the live transcript is off and the transcript is made after Stop. Models missing or downloading: the live view offers the download and says why there is no live text. The tooltip of the live switch says "PC".

## MCP

- [ ] Claude Desktop on Windows: paste the setup from Settings, Advanced, AI assistants, restart it, and confirm it lists the five Murmur tools.
- [ ] Claude Code (PowerShell and cmd): run the copied `claude mcp add murmur -- "<path>\murmur.exe" mcp` (a path with spaces) and confirm `claude mcp list` shows Murmur connected.
- [ ] With a real meeting recorded, ask the assistant to list meetings, read a transcript with speaker names, read a summary and search a word from it; compare with the Meetings page.
- [ ] With the first switch on and the dictation switch off, the assistant sees meetings and no `recent_dictations`. Turning the switch off while it is connected refuses the next call without restarting it. No console window flashes when the assistant starts `murmur.exe mcp`.

## Nightly and soak

- [ ] Start the `meetings windows` workflow by hand (`workflow_dispatch`, 180 minutes) and read the `soak` report: nothing lost to a kill beyond a second, no audio dropped, no piece sent twice, peak commit of processing under 1 GiB. Write the figures down; after three green nights remove `continue-on-error` from the `soak` job.
- [ ] Run `windows_meetings_soak` on a low-end laptop (`cargo test --lib -- --ignored --nocapture --exact meetings::soak::windows_meetings_soak`) and compare its peak commit and processing time with the runner's.
- [ ] Run the `loopback` job's two tests on a PC with a sound output. A hosted runner has none, so the job only warns; after three green runs on a machine that has an output, drop `continue-on-error` from it.
- [ ] A schedule only fires from the default branch. When the Windows work is synced to it, confirm the cron fires once.
- [ ] Parity with the Mac: run `meetings helper` by hand (`workflow_dispatch`) for the `parity-dialogue` artifact, commit both files under `src-tauri/src/meetings/fixtures/`, remove the `#[ignore]` of `the_turns_match_the_mac_helper_on_the_same_dialogue` and add it to the `cpu-model` filter of `windows.yml` (count raised by one).
- [ ] French needs a French voice. Check the `analysis` job's log for `no fr-FR voice`; on a PC with French voices run `two_french_voices_are_timed_and_told_apart` for real.
<!-- ci measurement -->
