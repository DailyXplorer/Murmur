@testable import HandyNative
import XCTest

final class NativeLaunchArgumentsTests: XCTestCase {
    func testPasteSmokeOutputEncodesJSONContractWithoutClipboardContents() throws {
        let output = NativePasteSmokeOutput(
            success: true,
            requestedText: "  hello  ",
            preparedText: "hello",
            pasteMethod: "ctrl_v",
            effectivePasteMethod: "ctrl_v",
            clipboardHandling: "dont_modify",
            pasteDelayMilliseconds: 60,
            startDelayMilliseconds: 500,
            appendTrailingSpace: false,
            autoSubmitKey: "cmd_enter",
            accessibilityTrusted: true,
            eventDispatchRequired: true,
            hadClipboardBefore: true,
            clipboardRestored: true,
            clipboardAfterEqualsPreparedText: false,
            clipboardAfterLength: 8,
            targetWindow: true,
            targetText: "hello",
            targetMatchesPreparedText: true,
            targetApplicationActive: true,
            targetWindowKey: true,
            targetFirstResponderClass: "NSTextView",
            targetInsertionDriver: "appkit_harness",
            activationRequestedProcessIdentifier: 12345,
            activationSucceeded: true,
            activatedApplicationBundleIdentifier: "computer.handy.Handy",
            activatedApplicationLocalizedName: "Handy",
            accessibilityWindowFocusSucceeded: true,
            frontmostApplicationProcessIdentifier: 12345,
            frontmostApplicationBundleIdentifier: "computer.handy.Handy",
            frontmostApplicationLocalizedName: "Handy"
        )
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]

        let data = try encoder.encode(output)
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])

        XCTAssertEqual(object["requestedText"] as? String, "  hello  ")
        XCTAssertEqual(object["success"] as? Bool, true)
        XCTAssertEqual(object["preparedText"] as? String, "hello")
        XCTAssertEqual(object["pasteMethod"] as? String, "ctrl_v")
        XCTAssertEqual(object["autoSubmitKey"] as? String, "cmd_enter")
        XCTAssertEqual(object["eventDispatchRequired"] as? Bool, true)
        XCTAssertEqual(object["clipboardRestored"] as? Bool, true)
        XCTAssertEqual(object["targetWindow"] as? Bool, true)
        XCTAssertEqual(object["targetMatchesPreparedText"] as? Bool, true)
        XCTAssertEqual(object["targetApplicationActive"] as? Bool, true)
        XCTAssertEqual(object["targetWindowKey"] as? Bool, true)
        XCTAssertEqual(object["targetFirstResponderClass"] as? String, "NSTextView")
        XCTAssertEqual(object["targetInsertionDriver"] as? String, "appkit_harness")
        XCTAssertEqual(object["activationRequestedProcessIdentifier"] as? Int, 12345)
        XCTAssertEqual(object["activationSucceeded"] as? Bool, true)
        XCTAssertEqual(object["activatedApplicationBundleIdentifier"] as? String, "computer.handy.Handy")
        XCTAssertEqual(object["accessibilityWindowFocusSucceeded"] as? Bool, true)
        XCTAssertEqual(object["frontmostApplicationProcessIdentifier"] as? Int, 12345)
        XCTAssertNil(object["clipboardBefore"])
        XCTAssertNil(object["clipboardAfter"])
    }

    func testPasteSmokeOutputCanEncodeExternalAccessibilityTargetText() throws {
        let output = NativePasteSmokeOutput(
            success: true,
            requestedText: "hello",
            preparedText: "hello",
            pasteMethod: "ctrl_v",
            effectivePasteMethod: "ctrl_v",
            clipboardHandling: "dont_modify",
            pasteDelayMilliseconds: 60,
            startDelayMilliseconds: 500,
            appendTrailingSpace: false,
            autoSubmitKey: nil,
            accessibilityTrusted: true,
            eventDispatchRequired: true,
            hadClipboardBefore: true,
            clipboardRestored: true,
            clipboardAfterEqualsPreparedText: false,
            clipboardAfterLength: 5,
            targetWindow: false,
            targetText: "hello",
            targetMatchesPreparedText: true,
            targetApplicationActive: nil,
            targetWindowKey: nil,
            targetFirstResponderClass: "AXTextArea",
            targetInsertionDriver: "accessibility_focused_element",
            activationRequestedProcessIdentifier: 23456,
            activationSucceeded: true,
            activatedApplicationBundleIdentifier: "com.apple.TextEdit",
            activatedApplicationLocalizedName: "TextEdit",
            accessibilityWindowFocusSucceeded: true,
            frontmostApplicationProcessIdentifier: 23456,
            frontmostApplicationBundleIdentifier: "com.apple.TextEdit",
            frontmostApplicationLocalizedName: "TextEdit"
        )
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]

        let data = try encoder.encode(output)
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])

        XCTAssertEqual(object["targetWindow"] as? Bool, false)
        XCTAssertEqual(object["targetText"] as? String, "hello")
        XCTAssertEqual(object["targetMatchesPreparedText"] as? Bool, true)
        XCTAssertEqual(object["targetFirstResponderClass"] as? String, "AXTextArea")
        XCTAssertEqual(object["targetInsertionDriver"] as? String, "accessibility_focused_element")
        XCTAssertEqual(object["activatedApplicationBundleIdentifier"] as? String, "com.apple.TextEdit")
        XCTAssertNil(object["clipboardBefore"])
        XCTAssertNil(object["clipboardAfter"])
    }

    func testExternalPasteTargetSmokeOutputEncodesJSONContract() throws {
        let output = NativeExternalPasteTargetSmokeOutput(
            success: true,
            outputPath: "/tmp/external-target/output.json",
            readyPath: "/tmp/external-target/ready",
            durationMilliseconds: 2_000,
            text: "hello",
            expectedText: "hello",
            matchedExpectedText: true,
            applicationActive: true,
            windowKey: true,
            firstResponderClass: "NSTextView"
        )
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]

        let data = try encoder.encode(output)
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])

        XCTAssertEqual(object["success"] as? Bool, true)
        XCTAssertEqual(object["outputPath"] as? String, "/tmp/external-target/output.json")
        XCTAssertEqual(object["readyPath"] as? String, "/tmp/external-target/ready")
        XCTAssertEqual(object["durationMilliseconds"] as? Int, 2_000)
        XCTAssertEqual(object["text"] as? String, "hello")
        XCTAssertEqual(object["expectedText"] as? String, "hello")
        XCTAssertEqual(object["matchedExpectedText"] as? Bool, true)
        XCTAssertEqual(object["firstResponderClass"] as? String, "NSTextView")
    }

    func testExternalPasteRoundTripSmokeOutputEncodesJSONContract() throws {
        let output = NativeExternalPasteRoundTripSmokeOutput(
            success: true,
            requestedText: "hello",
            expectedText: "hello ",
            paste: NativePasteSmokeOutput(
                success: true,
                requestedText: "hello",
                preparedText: "hello ",
                pasteMethod: "ctrl_v",
                effectivePasteMethod: "ctrl_v",
                clipboardHandling: "dont_modify",
                pasteDelayMilliseconds: 60,
                startDelayMilliseconds: 500,
                appendTrailingSpace: true,
                autoSubmitKey: nil,
                accessibilityTrusted: true,
                eventDispatchRequired: true,
                hadClipboardBefore: true,
                clipboardRestored: true,
                clipboardAfterEqualsPreparedText: false,
                clipboardAfterLength: 8,
                targetWindow: false,
                targetText: nil,
                targetMatchesPreparedText: false,
                targetApplicationActive: nil,
                targetWindowKey: nil,
                targetFirstResponderClass: nil,
                targetInsertionDriver: nil,
                activationRequestedProcessIdentifier: 456,
                activationSucceeded: true,
                activatedApplicationBundleIdentifier: "com.pais.handy",
                activatedApplicationLocalizedName: "Handy",
                accessibilityWindowFocusSucceeded: true,
                frontmostApplicationProcessIdentifier: 456,
                frontmostApplicationBundleIdentifier: "com.pais.handy",
                frontmostApplicationLocalizedName: "Handy"
            ),
            target: NativeExternalPasteTargetSmokeOutput(
                success: true,
                outputPath: "/tmp/external-target/output.json",
                readyPath: "/tmp/external-target/ready",
                durationMilliseconds: 5_000,
                text: "hello ",
                expectedText: "hello ",
                matchedExpectedText: true,
                applicationActive: true,
                windowKey: true,
                firstResponderClass: "NSTextView"
            ),
            targetProcessIdentifier: 456,
            targetTerminationStatus: 0,
            targetStandardOutput: "{\"success\":true}",
            targetStandardError: nil
        )
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]

        let data = try encoder.encode(output)
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        let paste = try XCTUnwrap(object["paste"] as? [String: Any])
        let target = try XCTUnwrap(object["target"] as? [String: Any])

        XCTAssertEqual(object["success"] as? Bool, true)
        XCTAssertEqual(object["requestedText"] as? String, "hello")
        XCTAssertEqual(object["expectedText"] as? String, "hello ")
        XCTAssertEqual(object["targetProcessIdentifier"] as? Int, 456)
        XCTAssertEqual(object["targetTerminationStatus"] as? Int, 0)
        XCTAssertEqual(paste["frontmostApplicationProcessIdentifier"] as? Int, 456)
        XCTAssertEqual(target["matchedExpectedText"] as? Bool, true)
        XCTAssertEqual(target["text"] as? String, "hello ")
    }

    func testGlobalShortcutSmokeOutputEncodesJSONContract() throws {
        let output = NativeGlobalShortcutSmokeOutput(
            success: true,
            requestedBindingID: ShortcutBinding.transcribeWithPostProcessID,
            requestedBinding: "option+shift+space",
            keyCode: 49,
            requiredFlagsRawValue: 655_360,
            accessibilityTrusted: true,
            eventTapRunning: true,
            eventPostSucceeded: true,
            pressedBindingIDs: [ShortcutBinding.transcribeWithPostProcessID],
            releasedBindingIDs: [ShortcutBinding.transcribeWithPostProcessID],
            observedPressed: true,
            observedReleased: true
        )
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]

        let data = try encoder.encode(output)
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])

        XCTAssertEqual(object["success"] as? Bool, true)
        XCTAssertEqual(object["requestedBindingID"] as? String, ShortcutBinding.transcribeWithPostProcessID)
        XCTAssertEqual(object["requestedBinding"] as? String, "option+shift+space")
        XCTAssertEqual(object["keyCode"] as? Int, 49)
        XCTAssertEqual(object["requiredFlagsRawValue"] as? Int, 655_360)
        XCTAssertEqual(object["accessibilityTrusted"] as? Bool, true)
        XCTAssertEqual(object["eventTapRunning"] as? Bool, true)
        XCTAssertEqual(object["eventPostSucceeded"] as? Bool, true)
        XCTAssertEqual(object["observedPressed"] as? Bool, true)
        XCTAssertEqual(object["observedReleased"] as? Bool, true)
        XCTAssertEqual(object["pressedBindingIDs"] as? [String], [ShortcutBinding.transcribeWithPostProcessID])
        XCTAssertEqual(object["releasedBindingIDs"] as? [String], [ShortcutBinding.transcribeWithPostProcessID])
    }

    func testGlobalShortcutRecordingSmokeOutputEncodesJSONContract() throws {
        let output = NativeGlobalShortcutRecordingSmokeOutput(
            success: true,
            requestedBindingID: ShortcutBinding.transcribeID,
            requestedBinding: "option+space",
            keyCode: 49,
            requiredFlagsRawValue: 524_288,
            accessibilityTrusted: true,
            microphonePermission: "granted",
            pushToTalk: true,
            holdDurationMilliseconds: 750,
            eventTapRunning: true,
            keyDownPostSucceeded: true,
            keyUpPostSucceeded: true,
            pressedBindingIDs: [ShortcutBinding.transcribeID],
            releasedBindingIDs: [ShortcutBinding.transcribeID],
            startedRecording: true,
            stoppedRecording: true,
            recordingSampleCount: 12_000,
            recordingSampleRate: 16_000,
            recordingDurationSeconds: 0.75,
            recordingMaxLevel: 0.42,
            levelObservationCount: 8,
            recordingHasAudibleSignal: true,
            recordingOutputPath: "/tmp/shortcut-recording.wav",
            recordingByteCount: 24_044,
            errorMessage: nil
        )
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]

        let data = try encoder.encode(output)
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])

        XCTAssertEqual(object["success"] as? Bool, true)
        XCTAssertEqual(object["requestedBindingID"] as? String, ShortcutBinding.transcribeID)
        XCTAssertEqual(object["microphonePermission"] as? String, "granted")
        XCTAssertEqual(object["pushToTalk"] as? Bool, true)
        XCTAssertEqual(object["holdDurationMilliseconds"] as? Int, 750)
        XCTAssertEqual(object["keyDownPostSucceeded"] as? Bool, true)
        XCTAssertEqual(object["keyUpPostSucceeded"] as? Bool, true)
        XCTAssertEqual(object["startedRecording"] as? Bool, true)
        XCTAssertEqual(object["stoppedRecording"] as? Bool, true)
        XCTAssertEqual(object["recordingSampleCount"] as? Int, 12_000)
        XCTAssertEqual(object["recordingSampleRate"] as? Double, 16_000)
        XCTAssertEqual(object["recordingHasAudibleSignal"] as? Bool, true)
        XCTAssertEqual(object["recordingOutputPath"] as? String, "/tmp/shortcut-recording.wav")
        XCTAssertEqual(object["recordingByteCount"] as? Int, 24_044)
        XCTAssertNil(object["errorMessage"])
    }

    func testGlobalShortcutRecordingSmokeOutputCanEncodeTranscriptionContract() throws {
        let output = NativeGlobalShortcutRecordingSmokeOutput(
            success: true,
            requestedBindingID: ShortcutBinding.transcribeWithPostProcessID,
            requestedBinding: "option+shift+space",
            keyCode: 49,
            requiredFlagsRawValue: 655_360,
            accessibilityTrusted: true,
            microphonePermission: "granted",
            pushToTalk: true,
            holdDurationMilliseconds: 1_250,
            eventTapRunning: true,
            keyDownPostSucceeded: true,
            keyUpPostSucceeded: true,
            pressedBindingIDs: [ShortcutBinding.transcribeWithPostProcessID],
            releasedBindingIDs: [ShortcutBinding.transcribeWithPostProcessID],
            startedRecording: true,
            stoppedRecording: true,
            recordingSampleCount: 20_000,
            recordingSampleRate: 16_000,
            recordingDurationSeconds: 1.25,
            recordingMaxLevel: 0.42,
            levelObservationCount: 14,
            recordingHasAudibleSignal: true,
            recordingOutputPath: "/tmp/shortcut-record-transcribe.wav",
            recordingByteCount: 40_044,
            transcription: NativeRecordTranscriptionSmokeOutput(
                outputPath: "/tmp/shortcut-record-transcribe.wav",
                requestedDurationMilliseconds: 1_250,
                capturedSampleCount: 20_000,
                processedSampleCount: 20_000,
                sampleRate: 16_000,
                durationSeconds: 1.25,
                maxLevel: 0.42,
                levelObservationCount: 14,
                byteCount: 40_044,
                microphoneName: nil,
                modelID: "tiny",
                language: "en",
                usedSelectedSettings: false,
                postProcessRequested: true,
                transcriptionText: "hello",
                outputText: "hello",
                historyEntryID: nil,
                recordingFileName: nil,
                paste: NativePasteSmokeOutput(
                    success: true,
                    requestedText: "hello",
                    preparedText: "hello",
                    pasteMethod: "ctrl_v",
                    effectivePasteMethod: "ctrl_v",
                    clipboardHandling: "dont_modify",
                    pasteDelayMilliseconds: 60,
                    startDelayMilliseconds: 500,
                    appendTrailingSpace: false,
                    autoSubmitKey: nil,
                    accessibilityTrusted: true,
                    eventDispatchRequired: true,
                    hadClipboardBefore: true,
                    clipboardRestored: true,
                    clipboardAfterEqualsPreparedText: false,
                    clipboardAfterLength: 5,
                    targetWindow: true,
                    targetText: "hello",
                    targetMatchesPreparedText: true,
                    targetApplicationActive: true,
                    targetWindowKey: true,
                    targetFirstResponderClass: "NSTextView",
                    targetInsertionDriver: "appkit_harness",
                    activationRequestedProcessIdentifier: nil,
                    activationSucceeded: nil,
                    activatedApplicationBundleIdentifier: nil,
                    activatedApplicationLocalizedName: nil,
                    accessibilityWindowFocusSucceeded: nil,
                    frontmostApplicationProcessIdentifier: nil,
                    frontmostApplicationBundleIdentifier: nil,
                    frontmostApplicationLocalizedName: nil
                )
            ),
            errorMessage: nil
        )
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]

        let data = try encoder.encode(output)
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        let transcription = try XCTUnwrap(object["transcription"] as? [String: Any])
        let paste = try XCTUnwrap(transcription["paste"] as? [String: Any])

        XCTAssertEqual(object["success"] as? Bool, true)
        XCTAssertEqual(object["requestedBindingID"] as? String, ShortcutBinding.transcribeWithPostProcessID)
        XCTAssertEqual(transcription["outputText"] as? String, "hello")
        XCTAssertEqual(transcription["postProcessRequested"] as? Bool, true)
        XCTAssertEqual(paste["targetMatchesPreparedText"] as? Bool, true)
        XCTAssertNil(paste["clipboardBefore"])
        XCTAssertNil(paste["clipboardAfter"])
    }

    func testRecordTranscriptionSmokeOutputEncodesJSONContract() throws {
        let output = NativeRecordTranscriptionSmokeOutput(
            outputPath: "/tmp/native-record-transcribe.wav",
            requestedDurationMilliseconds: 2_500,
            capturedSampleCount: 40_000,
            processedSampleCount: 20_000,
            sampleRate: 16_000,
            durationSeconds: 1.25,
            maxLevel: 0.42,
            levelObservationCount: 12,
            byteCount: 40_044,
            microphoneName: "Studio Mic",
            modelID: "tiny",
            language: "en",
            transcriptionText: "hello",
            outputText: "hello",
            historyEntryID: 42,
            recordingFileName: "recording-test.wav",
            paste: nil
        )
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]

        let data = try encoder.encode(output)
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])

        XCTAssertEqual(object["outputPath"] as? String, "/tmp/native-record-transcribe.wav")
        XCTAssertEqual(object["modelID"] as? String, "tiny")
        XCTAssertEqual(object["sampleRate"] as? Double, 16_000)
        XCTAssertEqual(object["transcriptionText"] as? String, "hello")
        XCTAssertEqual(object["historyEntryID"] as? Int, 42)
        XCTAssertEqual(object["recordingFileName"] as? String, "recording-test.wav")
        XCTAssertNil(object["paste"] as? [String: Any])
    }

    func testRecordTranscriptionSelectedSettingsSmokeOutputEncodesEngineContext() throws {
        let output = NativeRecordTranscriptionSmokeOutput(
            outputPath: "/tmp/native-record-transcribe.wav",
            requestedDurationMilliseconds: 2_500,
            capturedSampleCount: 40_000,
            processedSampleCount: 20_000,
            sampleRate: 16_000,
            durationSeconds: 1.25,
            maxLevel: 0.42,
            levelObservationCount: 12,
            byteCount: 40_044,
            microphoneName: "Studio Mic",
            modelID: "api:smoke:record-transcribe",
            modelDisplayName: "Smoke API Model",
            language: "fr",
            usedSelectedSettings: true,
            postProcessRequested: true,
            transcriptionText: "bonjour",
            outputText: "Bonjour.",
            historyEntryID: nil,
            recordingFileName: nil,
            paste: nil
        )
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]

        let data = try encoder.encode(output)
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])

        XCTAssertEqual(object["modelID"] as? String, "api:smoke:record-transcribe")
        XCTAssertEqual(object["modelDisplayName"] as? String, "Smoke API Model")
        XCTAssertEqual(object["language"] as? String, "fr")
        XCTAssertEqual(object["usedSelectedSettings"] as? Bool, true)
        XCTAssertEqual(object["postProcessRequested"] as? Bool, true)
        XCTAssertEqual(object["outputText"] as? String, "Bonjour.")
    }

    func testRecordTranscriptionPasteSmokeOutputEncodesNestedPasteContract() throws {
        let output = NativeRecordTranscriptionSmokeOutput(
            outputPath: "/tmp/native-record-transcribe.wav",
            requestedDurationMilliseconds: 2_500,
            capturedSampleCount: 40_000,
            processedSampleCount: 20_000,
            sampleRate: 16_000,
            durationSeconds: 1.25,
            maxLevel: 0.42,
            levelObservationCount: 12,
            byteCount: 40_044,
            microphoneName: "Studio Mic",
            modelID: "tiny",
            language: "en",
            transcriptionText: "hello",
            outputText: "hello",
            historyEntryID: nil,
            recordingFileName: nil,
            paste: NativePasteSmokeOutput(
                success: true,
                requestedText: "hello",
                preparedText: "hello",
                pasteMethod: "direct",
                effectivePasteMethod: "direct",
                clipboardHandling: "dont_modify",
                pasteDelayMilliseconds: 60,
                startDelayMilliseconds: 500,
                appendTrailingSpace: false,
                autoSubmitKey: nil,
                accessibilityTrusted: true,
                eventDispatchRequired: true,
                hadClipboardBefore: false,
                clipboardRestored: true,
                clipboardAfterEqualsPreparedText: false,
                clipboardAfterLength: nil,
                targetWindow: true,
                targetText: "hello",
                targetMatchesPreparedText: true,
                targetApplicationActive: true,
                targetWindowKey: true,
                targetFirstResponderClass: "NSTextView",
                targetInsertionDriver: "appkit_harness",
                activationRequestedProcessIdentifier: nil,
                activationSucceeded: nil,
                activatedApplicationBundleIdentifier: nil,
                activatedApplicationLocalizedName: nil,
                accessibilityWindowFocusSucceeded: nil,
                frontmostApplicationProcessIdentifier: nil,
                frontmostApplicationBundleIdentifier: nil,
                frontmostApplicationLocalizedName: nil
            )
        )
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]

        let data = try encoder.encode(output)
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        let paste = try XCTUnwrap(object["paste"] as? [String: Any])

        XCTAssertEqual(object["outputPath"] as? String, "/tmp/native-record-transcribe.wav")
        XCTAssertEqual(object["outputText"] as? String, "hello")
        XCTAssertEqual(paste["pasteMethod"] as? String, "direct")
        XCTAssertEqual(paste["targetMatchesPreparedText"] as? Bool, true)
        XCTAssertEqual(paste["targetInsertionDriver"] as? String, "appkit_harness")
        XCTAssertNil(paste["clipboardBefore"])
        XCTAssertNil(paste["clipboardAfter"])
    }

    func testTranscriptionPasteSmokeOutputEncodesNestedPasteContract() throws {
        let output = NativeTranscriptionSmokeOutput(
            transcriptionText: "hello",
            outputText: "hello",
            historyEntryID: nil,
            recordingFileName: nil,
            paste: NativePasteSmokeOutput(
                success: true,
                requestedText: "hello",
                preparedText: "hello",
                pasteMethod: "direct",
                effectivePasteMethod: "direct",
                clipboardHandling: "dont_modify",
                pasteDelayMilliseconds: 60,
                startDelayMilliseconds: 500,
                appendTrailingSpace: false,
                autoSubmitKey: nil,
                accessibilityTrusted: true,
                eventDispatchRequired: true,
                hadClipboardBefore: true,
                clipboardRestored: true,
                clipboardAfterEqualsPreparedText: false,
                clipboardAfterLength: 12,
                targetWindow: true,
                targetText: "hello",
                targetMatchesPreparedText: true,
                targetApplicationActive: true,
                targetWindowKey: true,
                targetFirstResponderClass: "NSTextView",
                targetInsertionDriver: "appkit_harness",
                activationRequestedProcessIdentifier: nil,
                activationSucceeded: nil,
                activatedApplicationBundleIdentifier: nil,
                activatedApplicationLocalizedName: nil,
                accessibilityWindowFocusSucceeded: nil,
                frontmostApplicationProcessIdentifier: nil,
                frontmostApplicationBundleIdentifier: nil,
                frontmostApplicationLocalizedName: nil
            )
        )
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]

        let data = try encoder.encode(output)
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        let paste = try XCTUnwrap(object["paste"] as? [String: Any])

        XCTAssertEqual(object["outputText"] as? String, "hello")
        XCTAssertEqual(paste["pasteMethod"] as? String, "direct")
        XCTAssertEqual(paste["targetMatchesPreparedText"] as? Bool, true)
        XCTAssertEqual(paste["targetInsertionDriver"] as? String, "appkit_harness")
        XCTAssertNil(paste["clipboardBefore"])
        XCTAssertNil(paste["clipboardAfter"])
    }

    func testTranscriptionExternalPasteRoundTripSmokeOutputEncodesNestedContract() throws {
        let externalPaste = NativeExternalPasteRoundTripSmokeOutput(
            success: true,
            requestedText: "hello",
            expectedText: "hello",
            paste: NativePasteSmokeOutput(
                success: true,
                requestedText: "hello",
                preparedText: "hello",
                pasteMethod: "ctrl_v",
                effectivePasteMethod: "ctrl_v",
                clipboardHandling: "dont_modify",
                pasteDelayMilliseconds: 60,
                startDelayMilliseconds: 500,
                appendTrailingSpace: false,
                autoSubmitKey: nil,
                accessibilityTrusted: true,
                eventDispatchRequired: true,
                hadClipboardBefore: false,
                clipboardRestored: true,
                clipboardAfterEqualsPreparedText: false,
                clipboardAfterLength: nil,
                targetWindow: false,
                targetText: nil,
                targetMatchesPreparedText: false,
                targetApplicationActive: nil,
                targetWindowKey: nil,
                targetFirstResponderClass: nil,
                targetInsertionDriver: nil,
                activationRequestedProcessIdentifier: 987,
                activationSucceeded: true,
                activatedApplicationBundleIdentifier: "com.pais.handy",
                activatedApplicationLocalizedName: "Handy",
                accessibilityWindowFocusSucceeded: true,
                frontmostApplicationProcessIdentifier: 987,
                frontmostApplicationBundleIdentifier: "com.pais.handy",
                frontmostApplicationLocalizedName: "Handy"
            ),
            target: NativeExternalPasteTargetSmokeOutput(
                success: true,
                outputPath: "/tmp/target.json",
                readyPath: "/tmp/ready",
                durationMilliseconds: 5_000,
                text: "hello",
                expectedText: "hello",
                matchedExpectedText: true,
                applicationActive: true,
                windowKey: true,
                firstResponderClass: "NSTextView"
            ),
            targetProcessIdentifier: 987,
            targetTerminationStatus: 0,
            targetStandardOutput: nil,
            targetStandardError: nil
        )
        let output = NativeTranscriptionSmokeOutput(
            transcriptionText: "hello",
            outputText: "hello",
            historyEntryID: nil,
            recordingFileName: nil,
            paste: externalPaste.paste,
            externalPaste: externalPaste
        )
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]

        let data = try encoder.encode(output)
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        let paste = try XCTUnwrap(object["paste"] as? [String: Any])
        let external = try XCTUnwrap(object["externalPaste"] as? [String: Any])
        let target = try XCTUnwrap(external["target"] as? [String: Any])

        XCTAssertEqual(object["outputText"] as? String, "hello")
        XCTAssertEqual(paste["frontmostApplicationProcessIdentifier"] as? Int, 987)
        XCTAssertEqual(external["success"] as? Bool, true)
        XCTAssertEqual(target["matchedExpectedText"] as? Bool, true)
    }

    func testSelectedSettingsTranscriptionSmokeOutputEncodesEngineContext() throws {
        let output = NativeTranscriptionSmokeOutput(
            modelID: "api:openai:gpt-4o-mini-transcribe",
            modelDisplayName: "OpenAI GPT-4o mini transcribe",
            language: "fr",
            usedSelectedSettings: true,
            postProcessRequested: true,
            transcriptionText: "bonjour",
            outputText: "Bonjour.",
            historyEntryID: nil,
            recordingFileName: nil,
            paste: nil
        )
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]

        let data = try encoder.encode(output)
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])

        XCTAssertEqual(object["modelID"] as? String, "api:openai:gpt-4o-mini-transcribe")
        XCTAssertEqual(object["modelDisplayName"] as? String, "OpenAI GPT-4o mini transcribe")
        XCTAssertEqual(object["language"] as? String, "fr")
        XCTAssertEqual(object["usedSelectedSettings"] as? Bool, true)
        XCTAssertEqual(object["postProcessRequested"] as? Bool, true)
        XCTAssertEqual(object["transcriptionText"] as? String, "bonjour")
        XCTAssertEqual(object["outputText"] as? String, "Bonjour.")
    }

    func testModelRuntimeSmokeOutputEncodesLoadedStateContract() throws {
        let output = NativeModelRuntimeSmokeOutput(
            modelID: "tiny",
            modelName: "Whisper Tiny",
            unloadTimeout: "never",
            unloadDelaySeconds: nil,
            waitMilliseconds: 0,
            explicitUnload: true,
            wasDownloaded: true,
            isDownloaded: true,
            byteCountBefore: 123,
            byteCountAfter: 456,
            loadedBefore: false,
            loadedAfterPrepare: true,
            loadedAfterWait: true,
            loadedAfterExplicitUnload: false,
            loadedModelIDsBefore: [],
            loadedModelIDsAfterPrepare: ["tiny"],
            loadedModelIDsAfterWait: ["tiny"],
            loadedModelIDsAfterExplicitUnload: []
        )
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]

        let data = try encoder.encode(output)
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])

        XCTAssertEqual(object["modelID"] as? String, "tiny")
        XCTAssertEqual(object["modelName"] as? String, "Whisper Tiny")
        XCTAssertEqual(object["unloadTimeout"] as? String, "never")
        XCTAssertNil(object["unloadDelaySeconds"])
        XCTAssertEqual(object["explicitUnload"] as? Bool, true)
        XCTAssertEqual(object["loadedBefore"] as? Bool, false)
        XCTAssertEqual(object["loadedAfterPrepare"] as? Bool, true)
        XCTAssertEqual(object["loadedAfterWait"] as? Bool, true)
        XCTAssertEqual(object["loadedAfterExplicitUnload"] as? Bool, false)
        XCTAssertEqual(object["loadedModelIDsAfterPrepare"] as? [String], ["tiny"])
        XCTAssertEqual(object["loadedModelIDsAfterExplicitUnload"] as? [String], [])
    }

    func testUpdateInstallScriptSmokeOutputEncodesScriptSafetyContract() throws {
        let output = NativeUpdateInstallScriptSmokeOutput(
            success: true,
            version: "0.9.0",
            protectedTargetParent: true,
            artifactFileName: "Handy_0.9.0_aarch64.app.zip",
            preparedAppBundleName: "Handy.app",
            installerScriptName: "install-handy-update.sh",
            installerScriptShellCheckStatus: 0,
            targetParentWritable: false,
            scriptContainsAdminBranch: true,
            scriptContainsWritableBranch: true,
            scriptContainsRollback: true,
            scriptContainsUserRelaunch: true,
            helperContainsRelaunch: false
        )
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]

        let data = try encoder.encode(output)
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])

        XCTAssertEqual(object["success"] as? Bool, true)
        XCTAssertEqual(object["version"] as? String, "0.9.0")
        XCTAssertEqual(object["protectedTargetParent"] as? Bool, true)
        XCTAssertEqual(object["targetParentWritable"] as? Bool, false)
        XCTAssertEqual(object["scriptContainsAdminBranch"] as? Bool, true)
        XCTAssertEqual(object["scriptContainsWritableBranch"] as? Bool, true)
        XCTAssertEqual(object["scriptContainsRollback"] as? Bool, true)
        XCTAssertEqual(object["scriptContainsUserRelaunch"] as? Bool, true)
        XCTAssertEqual(object["helperContainsRelaunch"] as? Bool, false)
        XCTAssertEqual(object["installerScriptShellCheckStatus"] as? Int, 0)
    }

    func testRemoteControlListenerSmokeOutputEncodesRoundTripContract() throws {
        let output = NativeRemoteControlListenerSmokeOutput(
            success: true,
            expectedCommand: "toggle-post-process",
            observedCommands: ["toggle-post-process"],
            receivedExpectedCommand: true,
            timeoutMilliseconds: 2_000,
            senderLaunchMethod: "launch-services",
            senderLaunchPath: "/tmp/handy-native-dist/Handy.app",
            bundleIdentifier: "com.pais.handy",
            listenerProcessIdentifier: 111,
            childTerminationStatus: 0,
            senderOutput: NativeRemoteControlSendSmokeOutput(
                command: "toggle-post-process",
                sent: true,
                peerAvailableBeforeSend: true,
                bundleIdentifier: "com.pais.handy",
                senderProcessIdentifier: 222
            )
        )
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]

        let data = try encoder.encode(output)
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        let sender = try XCTUnwrap(object["senderOutput"] as? [String: Any])

        XCTAssertEqual(object["success"] as? Bool, true)
        XCTAssertEqual(object["expectedCommand"] as? String, "toggle-post-process")
        XCTAssertEqual(object["observedCommands"] as? [String], ["toggle-post-process"])
        XCTAssertEqual(object["receivedExpectedCommand"] as? Bool, true)
        XCTAssertEqual(object["senderLaunchMethod"] as? String, "launch-services")
        XCTAssertEqual(object["senderLaunchPath"] as? String, "/tmp/handy-native-dist/Handy.app")
        XCTAssertEqual(object["bundleIdentifier"] as? String, "com.pais.handy")
        XCTAssertEqual(object["childTerminationStatus"] as? Int, 0)
        XCTAssertEqual(sender["sent"] as? Bool, true)
        XCTAssertEqual(sender["peerAvailableBeforeSend"] as? Bool, true)
        XCTAssertEqual(sender["command"] as? String, "toggle-post-process")
    }

    func testParseRuntimeLaunchFlags() {
        let arguments = NativeLaunchArguments.parse([
            "Handy",
            "--start-hidden",
            "--no-tray",
            "--debug",
            "--open-section",
            "advanced",
        ])

        XCTAssertTrue(arguments.startHidden)
        XCTAssertTrue(arguments.noTray)
        XCTAssertTrue(arguments.debug)
        XCTAssertEqual(arguments.initialSection, .advanced)
        XCTAssertNil(arguments.remoteCommand)
    }

    func testOpenSectionArgumentAcceptsProductLabelsAndSeparators() {
        XCTAssertEqual(
            NativeLaunchArguments.parse(["Handy", "--open-section=post-process"]).initialSection,
            .postProcessing
        )
        XCTAssertEqual(
            NativeLaunchArguments.parse(["Handy", "--open-section", "Post Process"]).initialSection,
            .postProcessing
        )
        XCTAssertNil(
            NativeLaunchArguments.parse(["Handy", "--open-section", "missing"]).initialSection
        )
    }

    func testRemoteCommandPriorityUsesFirstCommandArgument() {
        let arguments = NativeLaunchArguments.parse([
            "Handy",
            "--toggle-transcription",
            "--toggle-post-process",
            "--cancel",
        ])

        XCTAssertEqual(arguments.remoteCommand, .toggleTranscription)
    }

    func testRemoteCommandParsesPostProcessAndCancel() {
        XCTAssertEqual(
            NativeLaunchArguments.parse(["Handy", "--toggle-post-process"]).remoteCommand,
            .togglePostProcess
        )
        XCTAssertEqual(
            NativeLaunchArguments.parse(["Handy", "--cancel"]).remoteCommand,
            .cancel
        )
    }

    func testParsePasteSmokeArguments() {
        let arguments = NativeLaunchArguments.parse([
            "Handy",
            "--smoke-paste-text",
            "Bonjour Handy",
            "--smoke-paste-method=direct",
            "--smoke-clipboard-handling",
            "copy",
            "--smoke-paste-delay-ms=25",
            "--smoke-paste-start-delay-ms",
            "750",
            "--smoke-append-trailing-space",
            "--smoke-auto-submit",
            "cmd_enter",
            "--smoke-paste-target-window",
            "--smoke-paste-activate-pid=12345",
            "--smoke-output-json",
            "/tmp/paste-smoke.json",
        ])

        XCTAssertEqual(
            arguments.smokePasteRequest,
            NativePasteSmokeRequest(
                text: "Bonjour Handy",
                pasteMethod: .direct,
                clipboardHandling: .copyToClipboard,
                pasteDelayMilliseconds: 25,
                startDelayMilliseconds: 750,
                appendTrailingSpace: true,
                autoSubmitKey: .commandEnter,
                targetWindow: true,
                activationProcessIdentifier: 12345,
                outputPath: "/tmp/paste-smoke.json"
            )
        )
    }

    func testParseExternalPasteTargetSmokeArguments() {
        let arguments = NativeLaunchArguments.parse([
            "Handy",
            "--smoke-external-paste-target=/tmp/external-target/output.json",
            "--smoke-external-paste-ready",
            "/tmp/external-target/ready",
            "--smoke-external-paste-expected",
            "Bonjour Handy",
            "--smoke-external-paste-duration-ms=2500",
        ])

        XCTAssertEqual(
            arguments.smokeExternalPasteTargetRequest,
            NativeExternalPasteTargetSmokeRequest(
                outputPath: "/tmp/external-target/output.json",
                readyPath: "/tmp/external-target/ready",
                expectedText: "Bonjour Handy",
                durationMilliseconds: 2_500
            )
        )
    }

    func testParseExternalPasteRoundTripSmokeArguments() {
        let arguments = NativeLaunchArguments.parse([
            "Handy",
            "--smoke-external-paste-roundtrip",
            "--smoke-paste-text=hello",
            "--smoke-paste-method=cmd_v",
            "--smoke-clipboard-handling=restore",
            "--smoke-paste-delay-ms=80",
            "--smoke-paste-start-delay-ms=1200",
            "--smoke-append-trailing-space",
            "--smoke-external-paste-duration-ms=9000",
            "--smoke-output-json=/tmp/external-roundtrip.json",
        ])

        XCTAssertEqual(
            arguments.smokeExternalPasteRoundTripRequest,
            NativeExternalPasteRoundTripSmokeRequest(
                text: "hello",
                pasteMethod: .commandV,
                clipboardHandling: .dontModify,
                pasteDelayMilliseconds: 80,
                startDelayMilliseconds: 1_200,
                appendTrailingSpace: true,
                durationMilliseconds: 9_000,
                outputPath: "/tmp/external-roundtrip.json"
            )
        )
        XCTAssertNil(arguments.smokePasteRequest)
    }

    func testExternalPasteRoundTripCanUseFlagValueAsText() {
        let arguments = NativeLaunchArguments.parse([
            "Handy",
            "--smoke-external-paste-roundtrip=hello",
        ])

        XCTAssertEqual(arguments.smokeExternalPasteRoundTripRequest?.text, "hello")
        XCTAssertEqual(arguments.smokeExternalPasteRoundTripRequest?.durationMilliseconds, 5_000)
    }

    func testParseGlobalShortcutSmokeArguments() {
        let arguments = NativeLaunchArguments.parse([
            "Handy",
            "--smoke-global-shortcut-event-tap",
            "--smoke-global-shortcut-id",
            ShortcutBinding.transcribeWithPostProcessID,
            "--smoke-global-shortcut-binding=option+shift+space",
            "--smoke-output-json",
            "/tmp/global-shortcut-smoke.json",
        ])

        XCTAssertEqual(
            arguments.smokeGlobalShortcutRequest,
            NativeGlobalShortcutSmokeRequest(
                bindingID: ShortcutBinding.transcribeWithPostProcessID,
                binding: "option+shift+space",
                outputPath: "/tmp/global-shortcut-smoke.json"
            )
        )
    }

    func testGlobalShortcutSmokeDefaultsToTranscribeShortcut() {
        let arguments = NativeLaunchArguments.parse([
            "Handy",
            "--smoke-global-shortcut-event-tap",
        ])

        XCTAssertEqual(
            arguments.smokeGlobalShortcutRequest,
            NativeGlobalShortcutSmokeRequest(
                bindingID: ShortcutBinding.transcribeID,
                binding: "option+space",
                outputPath: nil
            )
        )
    }

    func testParseGlobalShortcutRecordingSmokeArguments() {
        let arguments = NativeLaunchArguments.parse([
            "Handy",
            "--smoke-global-shortcut-recording",
            "--smoke-global-shortcut-id=transcribe",
            "--smoke-global-shortcut-binding",
            "option+space",
            "--smoke-record-duration-ms",
            "750",
            "--smoke-record-microphone",
            "Studio Mic",
            "--smoke-global-shortcut-recording-output",
            "/tmp/shortcut-recording.wav",
            "--smoke-output-json=/tmp/shortcut-recording.json",
        ])

        XCTAssertEqual(
            arguments.smokeGlobalShortcutRecordingRequest,
            NativeGlobalShortcutRecordingSmokeRequest(
                bindingID: ShortcutBinding.transcribeID,
                binding: "option+space",
                durationMilliseconds: 750,
                microphoneName: "Studio Mic",
                recordingOutputPath: "/tmp/shortcut-recording.wav",
                outputPath: "/tmp/shortcut-recording.json"
            )
        )
    }

    func testParseGlobalShortcutRecordingTranscriptionPasteSmokeArguments() {
        let arguments = NativeLaunchArguments.parse([
            "Handy",
            "--smoke-global-shortcut-recording",
            "--smoke-global-shortcut-id=transcribe_with_post_process",
            "--smoke-global-shortcut-binding=option+shift+space",
            "--smoke-record-duration-ms=1250",
            "--smoke-global-shortcut-recording-output=/tmp/shortcut-record-transcribe.wav",
            "--smoke-transcribe-after-shortcut-recording",
            "--smoke-transcribe-selected-settings",
            "--smoke-transcribe-language=fr",
            "--smoke-post-process",
            "--smoke-record-history",
            "--smoke-external-paste-after-transcribe",
            "--smoke-paste-method=cmd_v",
            "--smoke-clipboard-handling=restore",
            "--smoke-paste-delay-ms=80",
            "--smoke-paste-start-delay-ms=1200",
            "--smoke-external-paste-duration-ms=9000",
            "--smoke-output-json=/tmp/shortcut-record-transcribe.json",
        ])

        XCTAssertEqual(
            arguments.smokeGlobalShortcutRecordingRequest,
            NativeGlobalShortcutRecordingSmokeRequest(
                bindingID: ShortcutBinding.transcribeWithPostProcessID,
                binding: "option+shift+space",
                durationMilliseconds: 1_250,
                microphoneName: nil,
                recordingOutputPath: "/tmp/shortcut-record-transcribe.wav",
                transcribeAfterRecording: true,
                modelID: "tiny",
                language: "fr",
                useSelectedSettings: true,
                postProcessRequested: true,
                recordHistory: true,
                pasteRequest: NativeTranscriptionPasteSmokeRequest(
                    pasteMethod: .commandV,
                    clipboardHandling: .dontModify,
                    pasteDelayMilliseconds: 80,
                    startDelayMilliseconds: 1_200,
                    appendTrailingSpace: false,
                    autoSubmitKey: nil,
                    targetWindow: false,
                    externalRoundTrip: true,
                    externalRoundTripDurationMilliseconds: 9_000
                ),
                outputPath: "/tmp/shortcut-record-transcribe.json"
            )
        )
    }

    func testGlobalShortcutRecordingTranscriptionSmokeCanRequestAppleSpeechModel() {
        let arguments = NativeLaunchArguments.parse([
            "Handy",
            "--smoke-global-shortcut-recording",
            "--smoke-transcribe-after-shortcut-recording",
            "--smoke-transcribe-model=apple-speech-native",
        ])

        XCTAssertEqual(arguments.smokeGlobalShortcutRecordingRequest?.modelID, TranscriptionAPIProvider.appleSpeechModelID)
        XCTAssertEqual(arguments.smokeGlobalShortcutRecordingRequest?.transcribeAfterRecording, true)
    }

    func testGlobalShortcutRecordingSmokeDefaultsToTranscribeShortcutAndClampsDuration() {
        let defaults = NativeLaunchArguments.parse([
            "Handy",
            "--smoke-global-shortcut-recording",
        ]).smokeGlobalShortcutRecordingRequest

        XCTAssertEqual(defaults?.bindingID, ShortcutBinding.transcribeID)
        XCTAssertEqual(defaults?.binding, "option+space")
        XCTAssertEqual(defaults?.durationMilliseconds, 1_000)
        XCTAssertNil(defaults?.microphoneName)
        XCTAssertNil(defaults?.recordingOutputPath)
        XCTAssertNil(defaults?.outputPath)

        XCTAssertEqual(
            NativeLaunchArguments.parse([
                "Handy",
                "--smoke-global-shortcut-recording",
                "--smoke-record-duration-ms=25",
            ]).smokeGlobalShortcutRecordingRequest?.durationMilliseconds,
            100
        )
        XCTAssertEqual(
            NativeLaunchArguments.parse([
                "Handy",
                "--smoke-global-shortcut-recording",
                "--smoke-record-duration-ms=30000",
            ]).smokeGlobalShortcutRecordingRequest?.durationMilliseconds,
            10_000
        )
    }

    func testExternalPasteTargetSmokeDefaultsAndClampsDuration() {
        XCTAssertEqual(
            NativeLaunchArguments.parse([
                "Handy",
                "--smoke-external-paste-target=/tmp/external-target/output.json",
            ]).smokeExternalPasteTargetRequest?.durationMilliseconds,
            5_000
        )
        XCTAssertEqual(
            NativeLaunchArguments.parse([
                "Handy",
                "--smoke-external-paste-target=/tmp/external-target/output.json",
                "--smoke-external-paste-duration-ms=100",
            ]).smokeExternalPasteTargetRequest?.durationMilliseconds,
            500
        )
        XCTAssertEqual(
            NativeLaunchArguments.parse([
                "Handy",
                "--smoke-external-paste-target=/tmp/external-target/output.json",
                "--smoke-external-paste-duration-ms=60000",
            ]).smokeExternalPasteTargetRequest?.durationMilliseconds,
            30_000
        )
    }

    func testPasteSmokeDefaultsAndClampsDelays() {
        let defaults = NativeLaunchArguments.parse([
            "Handy",
            "--smoke-paste-text=hello",
        ]).smokePasteRequest

        XCTAssertEqual(defaults?.pasteMethod, AppSettings.defaults.pasteMethod)
        XCTAssertEqual(defaults?.clipboardHandling, AppSettings.defaults.clipboardHandling)
        XCTAssertEqual(defaults?.pasteDelayMilliseconds, AppSettings.defaults.pasteDelayMilliseconds)
        XCTAssertEqual(defaults?.startDelayMilliseconds, 500)
        XCTAssertFalse(defaults?.appendTrailingSpace ?? true)
        XCTAssertNil(defaults?.autoSubmitKey)
        XCTAssertFalse(defaults?.targetWindow ?? true)
        XCTAssertNil(defaults?.activationProcessIdentifier)
        XCTAssertNil(defaults?.outputPath)

        let clamped = NativeLaunchArguments.parse([
            "Handy",
            "--smoke-paste-text=hello",
            "--smoke-paste-method",
            "cmd+shift+v",
            "--smoke-paste-delay-ms=-1",
            "--smoke-paste-start-delay-ms=30000",
            "--smoke-paste-activate-pid=-7",
            "--smoke-auto-submit=control-enter",
        ]).smokePasteRequest

        XCTAssertEqual(clamped?.pasteMethod, .commandShiftV)
        XCTAssertEqual(clamped?.pasteDelayMilliseconds, 0)
        XCTAssertEqual(clamped?.startDelayMilliseconds, 10_000)
        XCTAssertEqual(clamped?.autoSubmitKey, .controlEnter)
        XCTAssertFalse(clamped?.targetWindow ?? true)
        XCTAssertNil(clamped?.activationProcessIdentifier)
        XCTAssertNil(clamped?.outputPath)
    }

    func testParseSmokeTranscriptionArguments() {
        let arguments = NativeLaunchArguments.parse([
            "Handy",
            "--smoke-transcribe-file",
            "/tmp/audio.wav",
            "--smoke-transcribe-model=base",
            "--smoke-transcribe-language",
            "fr",
        ])

        XCTAssertEqual(
            arguments.smokeTranscriptionRequest,
            NativeTranscriptionSmokeRequest(
                filePath: "/tmp/audio.wav",
                modelID: "base",
                language: "fr",
                recordHistory: false,
                pasteRequest: nil,
                outputPath: nil
            )
        )
    }

    func testParseSmokeTranscriptionPasteArguments() {
        let arguments = NativeLaunchArguments.parse([
            "Handy",
            "--smoke-transcribe-file=/tmp/audio.wav",
            "--smoke-paste-after-transcribe",
            "--smoke-paste-method=direct",
            "--smoke-clipboard-handling=restore",
            "--smoke-paste-delay-ms=25",
            "--smoke-paste-start-delay-ms=750",
            "--smoke-append-trailing-space",
            "--smoke-auto-submit=cmd_enter",
            "--smoke-paste-target-window",
            "--smoke-output-json=/tmp/transcribe-paste.json",
        ])

        XCTAssertEqual(
            arguments.smokeTranscriptionRequest,
            NativeTranscriptionSmokeRequest(
                filePath: "/tmp/audio.wav",
                modelID: "tiny",
                language: nil,
                recordHistory: false,
                pasteRequest: NativeTranscriptionPasteSmokeRequest(
                    pasteMethod: .direct,
                    clipboardHandling: .dontModify,
                    pasteDelayMilliseconds: 25,
                    startDelayMilliseconds: 750,
                    appendTrailingSpace: true,
                    autoSubmitKey: .commandEnter,
                    targetWindow: true
                ),
                outputPath: "/tmp/transcribe-paste.json"
            )
        )
    }

    func testParseSmokeTranscriptionExternalPasteRoundTripArguments() {
        let arguments = NativeLaunchArguments.parse([
            "Handy",
            "--smoke-transcribe-file=/tmp/audio.wav",
            "--smoke-external-paste-after-transcribe",
            "--smoke-paste-method=cmd_v",
            "--smoke-clipboard-handling=restore",
            "--smoke-paste-delay-ms=25",
            "--smoke-paste-start-delay-ms=750",
            "--smoke-append-trailing-space",
            "--smoke-external-paste-duration-ms=9000",
            "--smoke-output-json=/tmp/transcribe-external-paste.json",
        ])

        XCTAssertEqual(
            arguments.smokeTranscriptionRequest,
            NativeTranscriptionSmokeRequest(
                filePath: "/tmp/audio.wav",
                modelID: "tiny",
                language: nil,
                recordHistory: false,
                pasteRequest: NativeTranscriptionPasteSmokeRequest(
                    pasteMethod: .commandV,
                    clipboardHandling: .dontModify,
                    pasteDelayMilliseconds: 25,
                    startDelayMilliseconds: 750,
                    appendTrailingSpace: true,
                    autoSubmitKey: nil,
                    targetWindow: false,
                    externalRoundTrip: true,
                    externalRoundTripDurationMilliseconds: 9_000
                ),
                outputPath: "/tmp/transcribe-external-paste.json"
            )
        )
    }

    func testSmokeTranscriptionDefaultsToTinyModel() {
        let arguments = NativeLaunchArguments.parse([
            "Handy",
            "--smoke-transcribe-file=/tmp/audio.wav",
        ])

        XCTAssertEqual(arguments.smokeTranscriptionRequest?.modelID, "tiny")
    }

    func testSmokeTranscriptionCanRequestAppleSpeechModel() {
        let arguments = NativeLaunchArguments.parse([
            "Handy",
            "--smoke-transcribe-file=/tmp/audio.wav",
            "--smoke-transcribe-model=apple-speech-native",
        ])

        XCTAssertEqual(arguments.smokeTranscriptionRequest?.modelID, TranscriptionAPIProvider.appleSpeechModelID)
    }

    func testSmokeTranscriptionCanRequestHistoryRecording() {
        let arguments = NativeLaunchArguments.parse([
            "Handy",
            "--smoke-transcribe-file=/tmp/audio.wav",
            "--smoke-record-history",
        ])

        XCTAssertTrue(arguments.smokeTranscriptionRequest?.recordHistory == true)
    }

    func testSmokeTranscriptionCanUseSelectedSettingsAndPostProcess() {
        let arguments = NativeLaunchArguments.parse([
            "Handy",
            "--smoke-transcribe-file=/tmp/audio.wav",
            "--smoke-transcribe-selected-settings",
            "--smoke-transcribe-language=de",
            "--smoke-post-process",
            "--smoke-output-json=/tmp/selected-settings-transcribe.json",
        ])

        XCTAssertEqual(
            arguments.smokeTranscriptionRequest,
            NativeTranscriptionSmokeRequest(
                filePath: "/tmp/audio.wav",
                modelID: "tiny",
                language: "de",
                useSelectedSettings: true,
                postProcessRequested: true,
                recordHistory: false,
                pasteRequest: nil,
                outputPath: "/tmp/selected-settings-transcribe.json"
            )
        )
    }

    func testParseRecordTranscriptionSmokeArguments() {
        let arguments = NativeLaunchArguments.parse([
            "Handy",
            "--smoke-record-transcribe",
            "/tmp/native-record-transcribe.wav",
            "--smoke-record-duration-ms=2500",
            "--smoke-record-microphone",
            "Studio Mic",
            "--smoke-transcribe-model=base",
            "--smoke-transcribe-language",
            "fr",
            "--smoke-record-history",
        ])

        XCTAssertEqual(
            arguments.smokeRecordTranscriptionRequest,
            NativeRecordTranscriptionSmokeRequest(
                outputPath: "/tmp/native-record-transcribe.wav",
                durationMilliseconds: 2_500,
                microphoneName: "Studio Mic",
                modelID: "base",
                language: "fr",
                recordHistory: true,
                pasteRequest: nil,
                outputJSONPath: nil
            )
        )
    }

    func testRecordTranscriptionSmokeCanUseSelectedSettingsAndPostProcess() {
        let arguments = NativeLaunchArguments.parse([
            "Handy",
            "--smoke-record-transcribe=/tmp/native-record-transcribe.wav",
            "--smoke-transcribe-selected-settings",
            "--smoke-transcribe-language=fr",
            "--smoke-post-process",
            "--smoke-output-json=/tmp/selected-settings-record-transcribe.json",
        ])

        XCTAssertEqual(
            arguments.smokeRecordTranscriptionRequest,
            NativeRecordTranscriptionSmokeRequest(
                outputPath: "/tmp/native-record-transcribe.wav",
                durationMilliseconds: 1_000,
                microphoneName: nil,
                modelID: "tiny",
                language: "fr",
                useSelectedSettings: true,
                postProcessRequested: true,
                recordHistory: false,
                pasteRequest: nil,
                outputJSONPath: "/tmp/selected-settings-record-transcribe.json"
            )
        )
    }

    func testRecordTranscriptionSmokeCanRequestAppleSpeechModel() {
        let arguments = NativeLaunchArguments.parse([
            "Handy",
            "--smoke-record-transcribe=/tmp/native-record-transcribe.wav",
            "--smoke-transcribe-model=apple-speech-native",
        ])

        XCTAssertEqual(arguments.smokeRecordTranscriptionRequest?.modelID, TranscriptionAPIProvider.appleSpeechModelID)
    }

    func testParseRecordTranscriptionPasteSmokeArguments() {
        let arguments = NativeLaunchArguments.parse([
            "Handy",
            "--smoke-record-transcribe=/tmp/native-record-transcribe.wav",
            "--smoke-paste-after-transcribe",
            "--smoke-paste-method=direct",
            "--smoke-clipboard-handling=restore",
            "--smoke-paste-delay-ms=25",
            "--smoke-paste-start-delay-ms=750",
            "--smoke-append-trailing-space",
            "--smoke-auto-submit=cmd_enter",
            "--smoke-paste-target-window",
            "--smoke-output-json=/tmp/record-transcribe-paste.json",
        ])

        XCTAssertEqual(
            arguments.smokeRecordTranscriptionRequest,
            NativeRecordTranscriptionSmokeRequest(
                outputPath: "/tmp/native-record-transcribe.wav",
                durationMilliseconds: 1_000,
                microphoneName: nil,
                modelID: "tiny",
                language: nil,
                recordHistory: false,
                pasteRequest: NativeTranscriptionPasteSmokeRequest(
                    pasteMethod: .direct,
                    clipboardHandling: .dontModify,
                    pasteDelayMilliseconds: 25,
                    startDelayMilliseconds: 750,
                    appendTrailingSpace: true,
                    autoSubmitKey: .commandEnter,
                    targetWindow: true
                ),
                outputJSONPath: "/tmp/record-transcribe-paste.json"
            )
        )
    }

    func testParseRecordTranscriptionExternalPasteRoundTripArguments() {
        let arguments = NativeLaunchArguments.parse([
            "Handy",
            "--smoke-record-transcribe=/tmp/native-record-transcribe.wav",
            "--smoke-external-paste-after-transcribe",
            "--smoke-paste-method=cmd_v",
            "--smoke-clipboard-handling=restore",
            "--smoke-paste-delay-ms=25",
            "--smoke-paste-start-delay-ms=750",
            "--smoke-external-paste-duration-ms=9000",
            "--smoke-output-json=/tmp/record-transcribe-external-paste.json",
        ])

        XCTAssertEqual(
            arguments.smokeRecordTranscriptionRequest,
            NativeRecordTranscriptionSmokeRequest(
                outputPath: "/tmp/native-record-transcribe.wav",
                durationMilliseconds: 1_000,
                microphoneName: nil,
                modelID: "tiny",
                language: nil,
                recordHistory: false,
                pasteRequest: NativeTranscriptionPasteSmokeRequest(
                    pasteMethod: .commandV,
                    clipboardHandling: .dontModify,
                    pasteDelayMilliseconds: 25,
                    startDelayMilliseconds: 750,
                    appendTrailingSpace: false,
                    autoSubmitKey: nil,
                    targetWindow: false,
                    externalRoundTrip: true,
                    externalRoundTripDurationMilliseconds: 9_000
                ),
                outputJSONPath: "/tmp/record-transcribe-external-paste.json"
            )
        )
    }

    func testRecordTranscriptionSmokeDefaultsAndClampsDuration() {
        XCTAssertEqual(
            NativeLaunchArguments.parse([
                "Handy",
                "--smoke-record-transcribe=/tmp/native-record-transcribe.wav",
            ]).smokeRecordTranscriptionRequest?.durationMilliseconds,
            1_000
        )
        XCTAssertEqual(
            NativeLaunchArguments.parse([
                "Handy",
                "--smoke-record-transcribe=/tmp/native-record-transcribe.wav",
                "--smoke-record-duration-ms=25",
            ]).smokeRecordTranscriptionRequest?.durationMilliseconds,
            100
        )
        XCTAssertEqual(
            NativeLaunchArguments.parse([
                "Handy",
                "--smoke-record-transcribe=/tmp/native-record-transcribe.wav",
                "--smoke-record-duration-ms=30000",
            ]).smokeRecordTranscriptionRequest?.durationMilliseconds,
            10_000
        )
    }

    func testParseAudioRecordingSmokeArguments() {
        let arguments = NativeLaunchArguments.parse([
            "Handy",
            "--smoke-record-audio",
            "/tmp/native-recording.wav",
            "--smoke-record-duration-ms=250",
            "--smoke-record-microphone",
            "Studio Mic",
        ])

        XCTAssertEqual(
            arguments.smokeAudioRecordingRequest,
            NativeAudioRecordingSmokeRequest(
                outputPath: "/tmp/native-recording.wav",
                durationMilliseconds: 250,
                microphoneName: "Studio Mic"
            )
        )
    }

    func testAudioRecordingSmokeDurationDefaultsAndClamps() {
        XCTAssertEqual(
            NativeLaunchArguments.parse([
                "Handy",
                "--smoke-record-audio=/tmp/native-recording.wav",
            ]).smokeAudioRecordingRequest?.durationMilliseconds,
            1_000
        )
        XCTAssertEqual(
            NativeLaunchArguments.parse([
                "Handy",
                "--smoke-record-audio=/tmp/native-recording.wav",
                "--smoke-record-duration-ms=25",
            ]).smokeAudioRecordingRequest?.durationMilliseconds,
            100
        )
        XCTAssertEqual(
            NativeLaunchArguments.parse([
                "Handy",
                "--smoke-record-audio=/tmp/native-recording.wav",
                "--smoke-record-duration-ms=30000",
            ]).smokeAudioRecordingRequest?.durationMilliseconds,
            10_000
        )
    }

    func testParseModelCacheSmokeArguments() {
        XCTAssertEqual(
            NativeLaunchArguments.parse(["Handy", "--smoke-model-cache-status", "tiny"]).smokeModelCacheRequest,
            NativeModelCacheSmokeRequest(modelID: "tiny", operation: .status)
        )
        XCTAssertEqual(
            NativeLaunchArguments.parse(["Handy", "--smoke-delete-model-cache=base"]).smokeModelCacheRequest,
            NativeModelCacheSmokeRequest(modelID: "base", operation: .delete)
        )
        XCTAssertEqual(
            NativeLaunchArguments.parse(["Handy", "--smoke-download-model-cache", "tiny"]).smokeModelCacheRequest,
            NativeModelCacheSmokeRequest(modelID: "tiny", operation: .download)
        )
    }

    func testParseModelRuntimeSmokeArguments() {
        let arguments = NativeLaunchArguments.parse([
            "Handy",
            "--smoke-model-runtime-state=base",
            "--smoke-model-runtime-unload-timeout=15s",
            "--smoke-model-runtime-wait-ms=200",
            "--smoke-model-runtime-explicit-unload",
            "--smoke-output-json=/tmp/model-runtime.json",
        ])

        XCTAssertEqual(
            arguments.smokeModelRuntimeRequest,
            NativeModelRuntimeSmokeRequest(
                modelID: "base",
                unloadTimeout: .sec15,
                waitMilliseconds: 200,
                explicitUnload: true,
                outputPath: "/tmp/model-runtime.json"
            )
        )
    }

    func testModelRuntimeSmokeDefaultsAndClampsWait() {
        let defaults = NativeLaunchArguments.parse([
            "Handy",
            "--smoke-model-runtime-state",
        ]).smokeModelRuntimeRequest

        XCTAssertEqual(defaults?.modelID, "tiny")
        XCTAssertEqual(defaults?.unloadTimeout, .never)
        XCTAssertEqual(defaults?.waitMilliseconds, 0)
        XCTAssertFalse(defaults?.explicitUnload ?? true)
        XCTAssertNil(defaults?.outputPath)

        XCTAssertEqual(
            NativeLaunchArguments.parse([
                "Handy",
                "--smoke-model-runtime-state=tiny",
                "--smoke-model-runtime-unload-timeout=immediate",
                "--smoke-model-runtime-wait-ms=-5",
            ]).smokeModelRuntimeRequest?.waitMilliseconds,
            0
        )
        XCTAssertEqual(
            NativeLaunchArguments.parse([
                "Handy",
                "--smoke-model-runtime-state=tiny",
                "--smoke-model-runtime-wait-ms=60000",
            ]).smokeModelRuntimeRequest?.waitMilliseconds,
            30_000
        )
        XCTAssertEqual(
            NativeLaunchArguments.parse([
                "Handy",
                "--smoke-model-runtime-state=tiny",
                "--smoke-model-runtime-unload-timeout=immediate",
            ]).smokeModelRuntimeRequest?.unloadTimeout,
            .immediately
        )
    }

    func testParseUpdateInstallScriptSmokeArguments() {
        let arguments = NativeLaunchArguments.parse([
            "Handy",
            "--smoke-update-install-script",
            "--smoke-update-version=1.2.3",
            "--smoke-update-protected-target",
            "--smoke-output-json=/tmp/update-install.json",
        ])

        XCTAssertEqual(
            arguments.smokeUpdateInstallScriptRequest,
            NativeUpdateInstallScriptSmokeRequest(
                version: "1.2.3",
                protectedTargetParent: true,
                outputPath: "/tmp/update-install.json"
            )
        )
    }

    func testUpdateInstallScriptSmokeDefaults() {
        let request = NativeLaunchArguments.parse([
            "Handy",
            "--smoke-update-install-script=true",
        ]).smokeUpdateInstallScriptRequest

        XCTAssertEqual(request?.version, "0.9.0")
        XCTAssertFalse(request?.protectedTargetParent ?? true)
        XCTAssertNil(request?.outputPath)
    }

    func testParseRemoteControlSmokeArguments() {
        let listener = NativeLaunchArguments.parse([
            "Handy",
            "--smoke-remote-control-listener",
            "--smoke-remote-control-command=post-process",
            "--smoke-remote-control-timeout-ms=2500",
            "--smoke-output-json=/tmp/remote-listener.json",
        ])

        XCTAssertEqual(
            listener.smokeRemoteControlListenerRequest,
            NativeRemoteControlListenerSmokeRequest(
                command: .togglePostProcess,
                timeoutMilliseconds: 2_500,
                outputPath: "/tmp/remote-listener.json"
            )
        )

        let launchServicesListener = NativeLaunchArguments.parse([
            "Handy",
            "--smoke-remote-control-listener",
            "--smoke-remote-control-launchservices",
        ])

        XCTAssertEqual(
            launchServicesListener.smokeRemoteControlListenerRequest,
            NativeRemoteControlListenerSmokeRequest(
                command: .toggleTranscription,
                timeoutMilliseconds: 5_000,
                senderLaunchMethod: .launchServices,
                outputPath: nil
            )
        )

        let sender = NativeLaunchArguments.parse([
            "Handy",
            "--smoke-remote-control-send=cancel",
            "--smoke-output-json=/tmp/remote-sender.json",
        ])

        XCTAssertEqual(
            sender.smokeRemoteControlSendRequest,
            NativeRemoteControlSendSmokeRequest(
                command: .cancel,
                outputPath: "/tmp/remote-sender.json"
            )
        )
    }

    func testRemoteControlSmokeDefaultsAndClampsTimeout() {
        let defaults = NativeLaunchArguments.parse([
            "Handy",
            "--smoke-remote-control-listener",
        ]).smokeRemoteControlListenerRequest

        XCTAssertEqual(defaults?.command, .toggleTranscription)
        XCTAssertEqual(defaults?.timeoutMilliseconds, 5_000)
        XCTAssertNil(defaults?.outputPath)

        XCTAssertEqual(
            NativeLaunchArguments.parse([
                "Handy",
                "--smoke-remote-control-listener",
                "--smoke-remote-control-timeout-ms=100",
            ]).smokeRemoteControlListenerRequest?.timeoutMilliseconds,
            500
        )
        XCTAssertEqual(
            NativeLaunchArguments.parse([
                "Handy",
                "--smoke-remote-control-listener",
                "--smoke-remote-control-timeout-ms=30000",
            ]).smokeRemoteControlListenerRequest?.timeoutMilliseconds,
            15_000
        )
        XCTAssertEqual(
            NativeLaunchArguments.parse([
                "Handy",
                "--smoke-remote-control-send",
                "--smoke-remote-control-command=post-process",
            ]).smokeRemoteControlSendRequest?.command,
            .togglePostProcess
        )
        XCTAssertEqual(
            NativeLaunchArguments.parse([
                "Handy",
                "--smoke-remote-control-send",
            ]).smokeRemoteControlSendRequest?.command,
            .toggleTranscription
        )
    }

    func testParseOverlaySmokeArguments() {
        XCTAssertEqual(
            NativeLaunchArguments.parse(["Handy", "--smoke-overlay-state", "recording"]).smokeOverlayState,
            .recording
        )
        XCTAssertEqual(
            NativeLaunchArguments.parse(["Handy", "--smoke-overlay-state=processing"]).smokeOverlayState,
            .processing
        )
        XCTAssertNil(
            NativeLaunchArguments.parse(["Handy", "--smoke-overlay-state", "missing"]).smokeOverlayState
        )
        XCTAssertEqual(
            NativeLaunchArguments.parse([
                "Handy",
                "--smoke-overlay-state",
                "recording",
                "--smoke-output-json",
                "/tmp/overlay.json",
                "--smoke-output-image",
                "/tmp/overlay.png",
            ]).smokeOverlayOutputPath,
            "/tmp/overlay.json"
        )
        XCTAssertEqual(
            NativeLaunchArguments.parse([
                "Handy",
                "--smoke-overlay-state",
                "recording",
                "--smoke-output-json",
                "/tmp/overlay.json",
                "--smoke-output-image",
                "/tmp/overlay.png",
            ]).smokeOverlayImageOutputPath,
            "/tmp/overlay.png"
        )
        XCTAssertNil(
            NativeLaunchArguments.parse(["Handy", "--smoke-output-json", "/tmp/overlay.json"]).smokeOverlayOutputPath
        )
        XCTAssertNil(
            NativeLaunchArguments.parse(["Handy", "--smoke-output-image", "/tmp/overlay.png"]).smokeOverlayImageOutputPath
        )
    }

    func testParseOnboardingSmokeArguments() {
        XCTAssertEqual(
            NativeLaunchArguments.parse(["Handy", "--smoke-onboarding-step", "permissions"]).smokeOnboardingStep,
            .permissions(returningUser: false)
        )
        XCTAssertEqual(
            NativeLaunchArguments.parse(["Handy", "--smoke-onboarding-step=model"]).smokeOnboardingStep,
            .model
        )
        XCTAssertNil(
            NativeLaunchArguments.parse(["Handy", "--smoke-onboarding-step", "missing"]).smokeOnboardingStep
        )
    }

    func testParsePermissionStatusSmokeArguments() {
        XCTAssertEqual(
            NativeLaunchArguments.parse([
                "Handy",
                "--smoke-permission-status",
                "--smoke-output-json=/tmp/permission-status.json",
            ]).smokePermissionStatusRequest,
            NativePermissionStatusSmokeRequest(outputPath: "/tmp/permission-status.json")
        )

        XCTAssertEqual(
            NativeLaunchArguments.parse([
                "Handy",
                "--smoke-permission-status=/tmp/direct-permission-status.json",
            ]).smokePermissionStatusRequest,
            NativePermissionStatusSmokeRequest(outputPath: "/tmp/direct-permission-status.json")
        )
    }

    func testParseReplacementReadinessSmokeArguments() {
        XCTAssertEqual(
            NativeLaunchArguments.parse([
                "Handy",
                "--smoke-replacement-readiness",
                "--smoke-replacement-readiness-strict",
                "--smoke-output-json=/tmp/replacement-readiness.json",
            ]).smokeReplacementReadinessRequest,
            NativeReplacementReadinessSmokeRequest(
                outputPath: "/tmp/replacement-readiness.json",
                strict: true
            )
        )

        XCTAssertEqual(
            NativeLaunchArguments.parse([
                "Handy",
                "--smoke-replacement-readiness=/tmp/direct-replacement-readiness.json",
            ]).smokeReplacementReadinessRequest,
            NativeReplacementReadinessSmokeRequest(
                outputPath: "/tmp/direct-replacement-readiness.json",
                strict: false
            )
        )
    }

    func testRemotePeerDetectionReturnsFalseWithoutBundleIdentifier() {
        XCTAssertFalse(
            NativeRemoteControlService.hasRunningPeer(
                bundleIdentifier: nil,
                currentProcessIdentifier: 123
            )
        )
    }
}
