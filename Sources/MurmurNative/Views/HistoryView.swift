import SwiftUI

struct HistoryView: View {
    @EnvironmentObject private var appModel: AppModel

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text("HISTORY")
                    .font(MurmurDesign.font(size: 12, weight: .medium))
                    .foregroundStyle(MurmurDesign.midGray)
                    .textCase(.uppercase)
                    .tracking(0.6)

                Spacer()

                Button {
                    appModel.openRecordingsFolder()
                } label: {
                    HStack(spacing: 8) {
                        MurmurHugeIcon(kind: .folderOpen, color: MurmurDesign.text, size: 16)
                        Text("Open Recordings Folder")
                    }
                }
                .buttonStyle(MurmurButtonStyle(variant: .secondary))
            }
            .padding(.horizontal, 16)

            VStack(spacing: 0) {
                if appModel.historyEntries.isEmpty {
                    Text("No transcriptions yet. Start recording to build your history!")
                        .font(MurmurDesign.font(size: 14))
                        .foregroundStyle(MurmurDesign.text.opacity(0.6))
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 12)
                } else {
                    ForEach(appModel.historyEntries) { entry in
                        HistoryEntryRow(entry: entry)
                            .environmentObject(appModel)

                        if entry.id != appModel.historyEntries.last?.id {
                            MurmurDivider()
                        }
                    }

                    if appModel.historyHasMore {
                        Color.clear
                            .frame(height: 1)
                            .onAppear {
                                appModel.loadMoreHistory()
                            }
                            .accessibilityHidden(true)
                    }
                }
            }
            .background(MurmurDesign.background)
            .clipShape(RoundedRectangle(cornerRadius: MurmurDesign.cornerRadius, style: .continuous))
            .overlay {
                RoundedRectangle(cornerRadius: MurmurDesign.cornerRadius, style: .continuous)
                    .stroke(MurmurDesign.midGray.opacity(0.2), lineWidth: 1)
            }
        }
        .frame(maxWidth: .infinity)
        .onAppear {
            appModel.reloadHistory()
        }
    }
}

private struct HistoryEntryRow: View {
    @Environment(\.colorScheme) private var colorScheme
    @EnvironmentObject private var appModel: AppModel
    @Environment(\.murmurTheme) private var murmurTheme

    let entry: HistoryEntry

    var body: some View {
        let isRetrying = appModel.retryingHistoryEntryIDs.contains(entry.id)
        let playback = playbackState

        VStack(alignment: .leading, spacing: 12) {
            HStack(alignment: .center) {
                Text(Self.dateFormatter.string(from: entry.date))
                    .font(MurmurDesign.font(size: 14, weight: .medium))
                    .foregroundStyle(MurmurDesign.text)

                Spacer()

                HStack(spacing: 0) {
                    Button {
                        appModel.copyHistoryText(entry)
                    } label: {
                        HistoryHugeIcon(
                            appModel.copiedHistoryEntryID == entry.id ? .check : .copy,
                            active: appModel.copiedHistoryEntryID == entry.id
                        )
                    }
                    .buttonStyle(HistoryIconButtonStyle(active: appModel.copiedHistoryEntryID == entry.id))
                    .disabled(isRetrying || !entry.hasTranscription)
                    .help("Copy transcription to clipboard")

                    Button {
                        appModel.toggleHistoryEntrySaved(id: entry.id)
                    } label: {
                        HistoryHugeIcon(.star, active: entry.saved, filled: entry.saved)
                    }
                    .buttonStyle(HistoryIconButtonStyle(active: entry.saved))
                    .disabled(isRetrying)
                    .help(entry.saved ? "Remove from saved" : "Save transcription")

                    Button {
                        appModel.retryHistoryEntryTranscription(id: entry.id)
                    } label: {
                        HistoryHugeIcon(.rotateLeft, active: isRetrying)
                            .rotationEffect(.degrees(isRetrying ? 360 : 0))
                            .animation(
                                isRetrying ? .linear(duration: 1).repeatForever(autoreverses: false) : .default,
                                value: isRetrying
                            )
                    }
                    .buttonStyle(HistoryIconButtonStyle(active: isRetrying))
                    .disabled(isRetrying || appModel.recordingState != .idle)
                    .help("Re-transcribe")

                    Button(role: .destructive) {
                        appModel.deleteHistoryEntry(id: entry.id)
                    } label: {
                        HistoryHugeIcon(.delete)
                    }
                    .buttonStyle(HistoryIconButtonStyle())
                    .disabled(isRetrying)
                    .help("Delete entry")
                }
            }

            Text(transcriptionDisplayText(isRetrying: isRetrying))
                .font(MurmurDesign.font(size: 14))
                .italic()
                .foregroundStyle(entry.hasTranscription && !isRetrying ? MurmurDesign.text.opacity(0.9) : MurmurDesign.text.opacity(0.4))
                .textSelection(.enabled)
                .fixedSize(horizontal: false, vertical: true)
                .padding(.bottom, 2)

            HStack(spacing: 12) {
                Button {
                    appModel.toggleHistoryAudioPlayback(entry)
                } label: {
                    if playback.isLoading(for: entry.id) {
                        ProgressView()
                            .controlSize(.small)
                            .frame(width: 20, height: 20)
                    } else {
                        MurmurHugeIcon(
                            kind: playback.isPlaying ? .pause : .play,
                            color: MurmurDesign.text.opacity(isRetrying ? 0.35 : 0.88),
                            size: 20
                        )
                    }
                }
                .buttonStyle(.plain)
                .foregroundStyle(MurmurDesign.text.opacity(isRetrying ? 0.35 : 0.88))
                .disabled(isRetrying)
                .help(playback.isPlaying ? "Pause" : "Play")

                HStack(spacing: 8) {
                    Text(AudioPlaybackTimeFormatter.formatted(playback.currentTime))
                        .font(MurmurDesign.font(size: 12))
                        .foregroundStyle(MurmurDesign.text.opacity(0.6))
                        .monospacedDigit()
                        .frame(width: 34, alignment: .leading)

                    HistoryProgressSlider(
                        entry: entry,
                        playback: playback,
                        disabled: isRetrying || playback.duration <= 0
                    )
                    .environmentObject(appModel)

                    Text(AudioPlaybackTimeFormatter.formatted(playback.duration))
                        .font(MurmurDesign.font(size: 12))
                        .foregroundStyle(MurmurDesign.text.opacity(0.6))
                        .monospacedDigit()
                        .frame(width: 34, alignment: .trailing)
                }
                .frame(maxWidth: .infinity)
            }
        }
        .padding(.horizontal, 16)
        .padding(.top, 8)
        .padding(.bottom, 20)
    }

    private func transcriptionDisplayText(isRetrying: Bool) -> String {
        if isRetrying {
            return "Transcribing..."
        }

        return entry.hasTranscription ? entry.transcriptionText : "Transcription failed. You can re-transcribe using the retry icon."
    }

    private var playbackState: AudioPlaybackState {
        if appModel.audioPlaybackState.entryID == entry.id {
            return appModel.audioPlaybackState
        }
        return .idle
    }

    private static let dateFormatter: DateFormatter = {
        let formatter = DateFormatter()
        formatter.dateStyle = .medium
        formatter.timeStyle = .short
        return formatter
    }()
}

private struct HistoryHugeIcon: View {
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.murmurTheme) private var murmurTheme

    let kind: MurmurHugeIconKind
    var active = false
    var filled = false

    init(_ kind: MurmurHugeIconKind, active: Bool = false, filled: Bool = false) {
        self.kind = kind
        self.active = active
        self.filled = filled
    }

    var body: some View {
        let color = active ? murmurTheme.logoPrimary(for: colorScheme) : MurmurDesign.text.opacity(0.5)
        MurmurHugeIcon(
            kind: kind,
            color: color,
            size: 16,
            fill: filled ? color : nil
        )
    }
}

private struct HistoryProgressSlider: View {
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.murmurTheme) private var murmurTheme
    @EnvironmentObject private var appModel: AppModel

    let entry: HistoryEntry
    let playback: AudioPlaybackState
    let disabled: Bool

    var body: some View {
        Slider(
            value: Binding(
                get: { playback.currentTime },
                set: { appModel.seekHistoryAudio(entry, to: $0) }
            ),
            in: 0...max(playback.duration, 0.01)
        )
        .tint(murmurTheme.logoPrimary(for: colorScheme))
        .disabled(disabled)
        .controlSize(.mini)
    }
}

private extension AudioPlaybackState {
    func isLoading(for entryID: Int64) -> Bool {
        self.entryID == entryID && isPlaying == false && duration <= 0
    }
}

private struct HistoryIconButtonStyle: ButtonStyle {
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.murmurTheme) private var murmurTheme
    @Environment(\.isEnabled) private var isEnabled

    var active = false

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.system(size: 15, weight: .medium))
            .foregroundStyle(active ? murmurTheme.logoPrimary(for: colorScheme) : MurmurDesign.text.opacity(configuration.isPressed ? 0.8 : 0.5))
            .frame(width: 28, height: 28)
            .contentShape(RoundedRectangle(cornerRadius: 6, style: .continuous))
            .opacity(isEnabled ? (configuration.isPressed ? 0.8 : 1) : 0.35)
    }
}
