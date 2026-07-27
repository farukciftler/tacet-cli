//
//  VoiceInput.swift
//  Tacet
//
//  In-app dictation. It turns the audio from the microphone into text ON DEVICE
//  and streams it live into the input field. There is NO automatic send — the
//  user sees what they are about to send, corrects it, and then sends.
//
//  WHY SpeechAnalyzer + DictationTranscriber:
//  iOS 26's SpeechAnalyzer pipeline runs entirely on device; there is no path for
//  the audio to reach an Apple server. The old `SFSpeechRecognizer` is not used at
//  all, because it can silently fall back to the server (when
//  requiresOnDeviceRecognition is forgotten). Of the modules, `DictationTranscriber`
//  was chosen: it uses the keyboard dictation's own on-device models, so it covers
//  all 9 languages of the interface — `SpeechTranscriber`'s narrow language list can
//  leave Turkish out. If the language is not supported the feature DOES NOT OPEN and
//  the user is told honestly.
//
//  This file makes no network call (the network monopoly: MCPClient /
//  WebSearchClient). If the model asset is missing, the system's own service
//  (AssetInventory) performs the download; the audio never leaves under any
//  condition.
//

// @preconcurrency: AVAudioConverter's conversion block has a `@Sendable` signature
// but is SYNCHRONOUS, called on the same thread; capturing the input buffer is safe.
// Without the marker, AVFAudio's missing Sendable conformances produced warnings.
@preconcurrency import AVFoundation
import Foundation
import Observation
import Speech
import UIKit

@MainActor
@Observable
final class VoiceInput {

    /// The listening state. `.preparing` is the permission/model step — because the
    /// model download can take a while on first launch, it appears as a separate state.
    enum State: Equatable {
        case idle
        case preparing
        case listening
    }

    /// The reason blocking dictation. The text is written in the view, not here; the
    /// service layer carries no localisation (repo style: the `Text(...)` lives in the view).
    enum Block: Equatable {
        case microphonePermission
        case speechPermission
        /// There is no on-device dictation model for the selected language — WE DO NOT
        /// FALL BACK to a server.
        case languageMissing
        case couldNotStart
    }

    private(set) var state: State = .idle
    /// The recognised text (finalised + currently being spoken). The view observes this.
    private(set) var transcribed: String = ""
    /// The block to show the user; the view sets it back to nil after reading it.
    var block: Block?

    var isRunning: Bool { state != .idle }
    var listening: Bool { state == .listening }

    /// Settings > the app page — if the permission is off we send the user there.
    static let settingsLink = URL(string: UIApplication.openSettingsURLString)

    // We are more generous while waiting for the first word: the microphone must not
    // close while the user presses the button and thinks about what to say.
    private static let firstSoundWait: TimeInterval = 5
    private static let silenceLimit: TimeInterval = 2.5

    private var engine: AVAudioEngine?
    private var evaluator: SpeechAnalyzer?
    private var writer: DictationTranscriber?
    private var streamEnd: AsyncStream<AnalyzerInput>.Continuation?
    private var resultTask: Task<Void, Never>?
    private var silenceTask: Task<Void, Never>?

    private var finalText = ""
    private var volatileText = ""
    private var lastSound = Date()
    private var spoke = false

    /// If the user presses the button again during the `await`s of startup, this counter
    /// prevents a half-built session from opening as `.listening`.
    private var generation = 0

    // MARK: - The outward-facing entry points

    func start() async {
        guard state == .idle else { return }
        state = .preparing
        finalText = ""
        volatileText = ""
        transcribed = ""
        spoke = false
        let myGeneration = generation

        do {
            try await requestPermissions()
            let local = try await recognitionLocale()

            let writer = DictationTranscriber(
                locale: local,
                contentHints: [.shortForm],
                transcriptionOptions: [.punctuation],
                reportingOptions: [.volatileResults],
                attributeOptions: []
            )
            try await prepareModel(writer, local: local)

            guard let format = await SpeechAnalyzer.bestAvailableAudioFormat(compatibleWith: [writer]) else {
                throw VoiceError.couldNotPrepare
            }

            let evaluator = SpeechAnalyzer(modules: [writer])
            let (stream, end) = AsyncStream<AnalyzerInput>.makeStream()
            try await evaluator.start(inputSequence: stream)

            // Every step up to here contained an `await`; if we were stopped in between we
            // tear down what we built ourselves and withdraw silently.
            guard myGeneration == generation, state == .preparing else {
                end.finish()
                await evaluator.cancelAndFinishNow()
                return
            }

            self.writer = writer
            self.evaluator = evaluator
            self.streamEnd = end
            listenForResults(writer)
            try startEngine(targetFormat: format, end: end)

            state = .listening
            lastSound = Date()
            watchSilence()
        } catch {
            let reason = (error as? VoiceError)?.block ?? .couldNotStart
            await stop()
            block = reason
        }
    }

    /// Start/stop is symmetric: the error path and the view disappearing both arrive here.
    /// The microphone is never left open.
    func stop() async {
        guard state != .idle else { return }
        state = .idle
        generation &+= 1

        // There is an `await` during teardown; the fields are cleared FIRST so that we do
        // not later delete what a new session started in the meantime has built.
        let engine = self.engine
        let evaluator = self.evaluator
        let end = self.streamEnd
        let resultTask = self.resultTask
        silenceTask?.cancel()
        silenceTask = nil
        self.engine = nil
        self.evaluator = nil
        self.writer = nil
        self.streamEnd = nil
        self.resultTask = nil

        if let engine {
            engine.inputNode.removeTap(onBus: 0)
            engine.stop()
        }
        end?.finish()

        // Finalise first, then cut the listening off, so the last final result is not missed.
        if let evaluator {
            try? await evaluator.finalizeAndFinishThroughEndOfInput()
        }
        resultTask?.cancel()

        // If a new listening session started while we waited, we do not close the session.
        if state == .idle {
            try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
        }
    }

    // MARK: - Permissions

    private enum VoiceError: Error {
        case noMicrophone
        case noSpeech
        case languageMissing
        case couldNotPrepare

        var block: Block {
            switch self {
            case .noMicrophone:     .microphonePermission
            case .noSpeech:         .speechPermission
            case .languageMissing:  .languageMissing
            case .couldNotPrepare:  .couldNotStart
            }
        }
    }

    /// The PermissionGate pattern: `denied` IS PERMANENT, it is not asked again every
    /// time — a block that routes straight to Settings is returned.
    private func requestPermissions() async throws {
        switch AVAudioApplication.shared.recordPermission {
        case .granted:
            break
        case .denied:
            throw VoiceError.noMicrophone
        default:
            let granted = await AVAudioApplication.requestRecordPermission()
            if !granted { throw VoiceError.noMicrophone }
        }

        // Speech recognition permission: even though the on-device pipeline never lets the
        // audio out, we do not recognise without asking the user. If the Info.plist key is
        // missing the prompt crashes the app; that is why the key's presence is checked
        // first.
        guard Bundle.main.object(forInfoDictionaryKey: "NSSpeechRecognitionUsageDescription") != nil else { return }

        switch SFSpeechRecognizer.authorizationStatus() {
        case .authorized:
            break
        case .denied, .restricted:
            throw VoiceError.noSpeech
        default:
            let outcome = await Self.requestSpeechPermission()
            if outcome != .authorized { throw VoiceError.noSpeech }
        }
    }

    private static func requestSpeechPermission() async -> SFSpeechRecognizerAuthorizationStatus {
        await withCheckedContinuation { continuation in
            // The system calls the callback once; the wrapper resumes once too.
            SFSpeechRecognizer.requestAuthorization { state in
                continuation.resume(returning: state)
            }
        }
    }

    // MARK: - Language and model

    /// The recognition language follows the reply/interface language the user chose;
    /// hardcoding "tr-TR" would condemn an English-speaking user to a Turkish model.
    private func recognitionLocale() async throws -> Locale {
        let preference = LanguagePreference.shared
        let code: String
        if !preference.replyLanguage.isEmpty {
            code = preference.replyLanguage
        } else if !preference.uiLanguage.isEmpty {
            code = preference.uiLanguage
        } else {
            code = Locale.preferredLanguages.first ?? Locale.current.identifier
        }
        guard let matched = await DictationTranscriber.supportedLocale(equivalentTo: Locale(identifier: code)) else {
            throw VoiceError.languageMissing
        }
        return matched
    }

    private func prepareModel(_ writer: DictationTranscriber, local: Locale) async throws {
        let target = local.identifier(.bcp47)
        let installed = await DictationTranscriber.installedLocales
        if installed.contains(where: { $0.identifier(.bcp47) == target }) { return }

        // If the model is missing the system service downloads it (not our network code).
        guard let request = try await AssetInventory.assetInstallationRequest(supporting: [writer]) else {
            throw VoiceError.languageMissing
        }
        try await request.downloadAndInstall()
    }

    // MARK: - The result stream

    private func listenForResults(_ writer: DictationTranscriber) {
        resultTask = Task { [weak self] in
            do {
                for try await outcome in writer.results {
                    guard let self else { return }
                    let chunk = String(outcome.text.characters)
                    if outcome.isFinal {
                        self.finalText += chunk
                        self.volatileText = ""
                    } else {
                        self.volatileText = chunk
                    }
                    self.transcribed = self.finalText + self.volatileText
                    self.lastSound = Date()
                    self.spoke = true
                }
            } catch {
                guard let self, self.state != .idle else { return }
                await self.stop()
                self.block = .couldNotStart
            }
        }
    }

    /// Stop by itself on silence: if the user forgets to press the button, the microphone
    /// must not stay open. The criterion is the result stream — while there is speech
    /// results arrive, when it stops they do not.
    private func watchSilence() {
        silenceTask = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: .milliseconds(500))
                guard !Task.isCancelled, let self, self.state == .listening else { return }
                let limit = self.spoke ? Self.silenceLimit : Self.firstSoundWait
                if Date().timeIntervalSince(self.lastSound) > limit {
                    await self.stop()
                    return
                }
            }
        }
    }

    // MARK: - The audio engine

    private func startEngine(targetFormat: AVAudioFormat,
                             end: AsyncStream<AnalyzerInput>.Continuation) throws {
        let session = AVAudioSession.sharedInstance()
        // `.measurement`: signal processing (AGC/EQ) is turned off, recognition accuracy
        // goes up.
        try session.setCategory(.record, mode: .measurement, options: [])
        try session.setActive(true, options: .notifyOthersOnDeactivation)

        let engine = AVAudioEngine()
        let input = engine.inputNode
        let inputFormat = input.outputFormat(forBus: 0)
        guard inputFormat.sampleRate > 0 else { throw VoiceError.couldNotPrepare }

        let converter = SpeechTranscriber(target: targetFormat)
        // The tap runs on the real-time audio thread: nothing touching MainActor is
        // captured, only Sendable local values.
        input.installTap(onBus: 0, bufferSize: 4096, format: inputFormat) { buffer, _ in
            guard let ready = converter.transform(buffer) else { return }
            end.yield(AnalyzerInput(buffer: ready))
        }

        engine.prepare()
        do {
            try engine.start()
        } catch {
            input.removeTap(onBus: 0)
            try? session.setActive(false, options: .notifyOthersOnDeactivation)
            throw VoiceError.couldNotPrepare
        }
        self.engine = engine
    }
}

/// Converts the microphone's native format into the format the analyzer wants.
/// It is deliberately un-isolated because it is called from the real-time audio thread;
/// only a single tap closure uses the instance.
private nonisolated final class SpeechTranscriber: @unchecked Sendable {
    private let target: AVAudioFormat
    private var converter: AVAudioConverter?

    init(target: AVAudioFormat) {
        self.target = target
    }

    func transform(_ buffer: AVAudioPCMBuffer) -> AVAudioPCMBuffer? {
        let source = buffer.format
        if source == target { return buffer }

        if converter == nil || converter?.inputFormat != source {
            let new = AVAudioConverter(from: source, to: target)
            // Priming is off: we give up the quality of the first samples to avoid a
            // timestamp drift in the live stream.
            new?.primeMethod = AVAudioConverterPrimeMethod.none
            converter = new
        }
        guard let converter else { return nil }

        let ratio = target.sampleRate / source.sampleRate
        let capacity = AVAudioFrameCount((Double(buffer.frameLength) * ratio).rounded(.up))
        guard capacity > 0, let output = AVAudioPCMBuffer(pcmFormat: target, frameCapacity: capacity) else { return nil }

        var delivered = false
        var error: NSError?
        converter.convert(to: output, error: &error) { _, state in
            // A single-buffer conversion: on the second request we say "no data now".
            if delivered {
                state.pointee = .noDataNow
                return nil
            }
            delivered = true
            state.pointee = .haveData
            return buffer
        }
        guard error == nil, output.frameLength > 0 else { return nil }
        return output
    }
}
