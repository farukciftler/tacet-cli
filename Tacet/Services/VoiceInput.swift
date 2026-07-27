//
//  SesGirisi.swift
//  Tacet
//
//  Uygulama içi dikte. Mikrofondan gelen sesi CİHAZ ÜSTÜNDE yazıya çevirir ve
//  giriş alanına canlı olarak akıtır. Otomatik gönderim YOKTUR — kullanıcı ne
//  göndereceğini görür, düzeltir, sonra gönderir.
//
//  NEDEN SpeechAnalyzer + DictationTranscriber:
//  iOS 26'nın SpeechAnalyzer boru hattı tümüyle cihaz üstünde çalışır; sesin
//  Apple sunucusuna gitme yolu yoktur. Eski `SFSpeechRecognizer` ise sessizce
//  sunucuya düşebildiği için (requiresOnDeviceRecognition unutulduğunda) hiç
//  kullanılmıyor. Modüllerden `DictationTranscriber` seçildi: klavye diktesinin
//  kendi cihaz-üstü modellerini kullanır, yani arayüzün 9 dilinin tamamını
//  kapsar — `SpeechTranscriber`ın dar dil listesi Türkçeyi dışarıda bırakabilir.
//  Dil desteklenmiyorsa özellik AÇILMAZ ve kullanıcıya dürüstçe söylenir.
//
//  Bu dosya ağ çağrısı yapmaz (ağ tekeli: MCPIstemcisi / WebAramaIstemcisi).
//  Model varlığı eksikse indirmeyi sistemin kendi servisi (AssetInventory)
//  yürütür; ses hiçbir koşulda dışarı çıkmaz.
//

// @preconcurrency: AVAudioConverter'ın dönüştürme bloğu `@Sendable` imzalı ama
// SENKRON, aynı iş parçacığında çağrılır; girdi tamponunu yakalamak güvenlidir.
// İşaret olmadan AVFAudio'nun Sendable eksikleri uyarı üretiyordu.
@preconcurrency import AVFoundation
import Foundation
import Observation
import Speech
import UIKit

@MainActor
@Observable
final class VoiceInput {

    /// Dinleme durumu. `.preparing` izin/model adımıdır — ilk açılışta model
    /// indirmesi sürebildiği için ayrı bir durum olarak görünür.
    enum State: Equatable {
        case idle
        case preparing
        case listening
    }

    /// Dikteyi engelleyen sebep. Metin burada değil görünümde yazılır; servis
    /// katmanı yerelleştirme taşımaz (repo üslubu: `Text("Türkçe")` görünümde).
    enum Block: Equatable {
        case microphonePermission
        case speechPermission
        /// Seçili dilde cihaz-üstü dikte modeli yok — sunucuya DÜŞMEYİZ.
        case languageMissing
        case couldNotStart
    }

    private(set) var state: State = .idle
    /// Tanınan metin (kesinleşen + o an konuşulan). Görünüm bunu izler.
    private(set) var transcribed: String = ""
    /// Kullanıcıya gösterilecek engel; görünüm okuduktan sonra nil'e çeker.
    var block: Block?

    var isRunning: Bool { state != .idle }
    var listening: Bool { state == .listening }

    /// Ayarlar > uygulama sayfası — izin kapalıysa kullanıcıyı oraya yollarız.
    static let settingsLink = URL(string: UIApplication.openSettingsURLString)

    // İlk kelime beklenirken daha cömert davranırız: kullanıcı düğmeye basıp
    // ne diyeceğini düşünürken mikrofon kapanmasın.
    private static let ilkSesBeklemesi: TimeInterval = 5
    private static let sessizlikSiniri: TimeInterval = 2.5

    private var engine: AVAudioEngine?
    private var evaluator: SpeechAnalyzer?
    private var writer: DictationTranscriber?
    private var akisUcu: AsyncStream<AnalyzerInput>.Continuation?
    private var sonucGorevi: Task<Void, Never>?
    private var sessizlikGorevi: Task<Void, Never>?

    private var kesinMetin = ""
    private var geciciMetin = ""
    private var sonSes = Date()
    private var konusuldu = false

    /// Başlatma sırasındaki `await`ler boyunca kullanıcı düğmeye tekrar basarsa
    /// yarı kurulmuş bir oturumun `.dinliyor` diye açılmasını bu sayaç önler.
    private var kusak = 0

    // MARK: - Dışarı açılan uçlar

    func start() async {
        guard state == .idle else { return }
        state = .preparing
        kesinMetin = ""
        geciciMetin = ""
        transcribed = ""
        konusuldu = false
        let benimKusagim = kusak

        do {
            try await izinAl()
            let local = try await tanimaYereli()

            let writer = DictationTranscriber(
                locale: local,
                contentHints: [.shortForm],
                transcriptionOptions: [.punctuation],
                reportingOptions: [.volatileResults],
                attributeOptions: []
            )
            try await modeliHazirla(writer, local: local)

            guard let format = await SpeechAnalyzer.bestAvailableAudioFormat(compatibleWith: [writer]) else {
                throw VoiceError.hazirlanamadi
            }

            let evaluator = SpeechAnalyzer(modules: [writer])
            let (stream, uc) = AsyncStream<AnalyzerInput>.makeStream()
            try await evaluator.start(inputSequence: stream)

            // Buraya kadarki her adım `await` içeriyordu; arada durdurulduysak
            // kurduğumuzu kendi elimizle toplayıp sessizce çekiliriz.
            guard benimKusagim == kusak, state == .preparing else {
                uc.finish()
                await evaluator.cancelAndFinishNow()
                return
            }

            self.writer = writer
            self.evaluator = evaluator
            self.akisUcu = uc
            sonuclariDinle(writer)
            try motoruBaslat(hedefBicim: format, uc: uc)

            state = .listening
            sonSes = Date()
            sessizlikGozcusu()
        } catch {
            let reason = (error as? VoiceError)?.block ?? .couldNotStart
            await stop()
            block = reason
        }
    }

    /// Başlat/durdur simetrik: hata yolunda da, görünüm kaybolduğunda da buraya
    /// gelinir. Mikrofon açık kalmaz.
    func stop() async {
        guard state != .idle else { return }
        state = .idle
        kusak &+= 1

        // Kapatma sırasında `await` var; alanlar ÖNCE boşaltılır ki bu arada
        // başlayan yeni bir oturumun kurduklarını sonradan silmeyelim.
        let engine = self.engine
        let evaluator = self.evaluator
        let uc = self.akisUcu
        let sonucGorevi = self.sonucGorevi
        sessizlikGorevi?.cancel()
        sessizlikGorevi = nil
        self.engine = nil
        self.evaluator = nil
        self.writer = nil
        self.akisUcu = nil
        self.sonucGorevi = nil

        if let engine {
            engine.inputNode.removeTap(onBus: 0)
            engine.stop()
        }
        uc?.finish()

        // Son kesin sonucu kaçırmamak için önce bitir, sonra dinlemeyi kes.
        if let evaluator {
            try? await evaluator.finalizeAndFinishThroughEndOfInput()
        }
        sonucGorevi?.cancel()

        // Beklerken yeni bir dinleme başladıysa oturumu kapatmayız.
        if state == .idle {
            try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
        }
    }

    // MARK: - İzinler

    private enum VoiceError: Error {
        case mikrofonYok
        case konusmaYok
        case languageMissing
        case hazirlanamadi

        var block: Block {
            switch self {
            case .mikrofonYok:   .microphonePermission
            case .konusmaYok:    .speechPermission
            case .languageMissing:        .languageMissing
            case .hazirlanamadi: .couldNotStart
            }
        }
    }

    /// IzinKapisi deseni: `denied` KALICIDIR, her seferinde yeniden sorulmaz —
    /// doğrudan Ayarlar'a yönlendiren engel döner.
    private func izinAl() async throws {
        switch AVAudioApplication.shared.recordPermission {
        case .granted:
            break
        case .denied:
            throw VoiceError.mikrofonYok
        default:
            let verildi = await AVAudioApplication.requestRecordPermission()
            if !verildi { throw VoiceError.mikrofonYok }
        }

        // Konuşma tanıma izni: cihaz-üstü boru hattı sesi dışarı çıkarmasa da
        // kullanıcıya sormadan tanıma yapmayız. Info.plist anahtarı yoksa istem
        // uygulamayı düşürür; o yüzden anahtarın varlığı önce kontrol edilir.
        guard Bundle.main.object(forInfoDictionaryKey: "NSSpeechRecognitionUsageDescription") != nil else { return }

        switch SFSpeechRecognizer.authorizationStatus() {
        case .authorized:
            break
        case .denied, .restricted:
            throw VoiceError.konusmaYok
        default:
            let outcome = await Self.konusmaIzniIste()
            if outcome != .authorized { throw VoiceError.konusmaYok }
        }
    }

    private static func konusmaIzniIste() async -> SFSpeechRecognizerAuthorizationStatus {
        await withCheckedContinuation { devam in
            // Sistem geri çağrımı tek kez çağırır; sarmalayıcı da tek resume eder.
            SFSpeechRecognizer.requestAuthorization { state in
                devam.resume(returning: state)
            }
        }
    }

    // MARK: - Dil ve model

    /// Tanıma dili kullanıcının seçtiği yanıt/arayüz dilini izler; sabit "tr-TR"
    /// yazmak İngilizce konuşan kullanıcıyı Türkçe modele mahkûm ederdi.
    private func tanimaYereli() async throws -> Locale {
        let preference = LanguagePreference.shared
        let code: String
        if !preference.replyLanguage.isEmpty {
            code = preference.replyLanguage
        } else if !preference.uiLanguage.isEmpty {
            code = preference.uiLanguage
        } else {
            code = Locale.preferredLanguages.first ?? Locale.current.identifier
        }
        guard let eslesen = await DictationTranscriber.supportedLocale(equivalentTo: Locale(identifier: code)) else {
            throw VoiceError.languageMissing
        }
        return eslesen
    }

    private func modeliHazirla(_ writer: DictationTranscriber, local: Locale) async throws {
        let target = local.identifier(.bcp47)
        let installed = await DictationTranscriber.installedLocales
        if installed.contains(where: { $0.identifier(.bcp47) == target }) { return }

        // Model eksikse sistem servisi indirir (bizim ağ kodumuz değil).
        guard let request = try await AssetInventory.assetInstallationRequest(supporting: [writer]) else {
            throw VoiceError.languageMissing
        }
        try await request.downloadAndInstall()
    }

    // MARK: - Sonuç akışı

    private func sonuclariDinle(_ writer: DictationTranscriber) {
        sonucGorevi = Task { [weak self] in
            do {
                for try await outcome in writer.results {
                    guard let self else { return }
                    let chunk = String(outcome.text.characters)
                    if outcome.isFinal {
                        self.kesinMetin += chunk
                        self.geciciMetin = ""
                    } else {
                        self.geciciMetin = chunk
                    }
                    self.transcribed = self.kesinMetin + self.geciciMetin
                    self.sonSes = Date()
                    self.konusuldu = true
                }
            } catch {
                guard let self, self.state != .idle else { return }
                await self.stop()
                self.block = .couldNotStart
            }
        }
    }

    /// Sessizlikte kendiliğinden dur: kullanıcı düğmeye basmayı unutursa
    /// mikrofon açık kalmasın. Ölçüt sonuç akışıdır — konuşma varken sonuç
    /// gelir, sustuğunda gelmez.
    private func sessizlikGozcusu() {
        sessizlikGorevi = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: .milliseconds(500))
                guard !Task.isCancelled, let self, self.state == .listening else { return }
                let limit = self.konusuldu ? Self.sessizlikSiniri : Self.ilkSesBeklemesi
                if Date().timeIntervalSince(self.sonSes) > limit {
                    await self.stop()
                    return
                }
            }
        }
    }

    // MARK: - Ses motoru

    private func motoruBaslat(hedefBicim: AVAudioFormat, uc: AsyncStream<AnalyzerInput>.Continuation) throws {
        let session = AVAudioSession.sharedInstance()
        // `.measurement`: sinyal işleme (AGC/EQ) kapanır, tanıma doğruluğu artar.
        try session.setCategory(.record, mode: .measurement, options: [])
        try session.setActive(true, options: .notifyOthersOnDeactivation)

        let engine = AVAudioEngine()
        let input = engine.inputNode
        let girdiBicimi = input.outputFormat(forBus: 0)
        guard girdiBicimi.sampleRate > 0 else { throw VoiceError.hazirlanamadi }

        let donusturucu = SpeechTranscriber(target: hedefBicim)
        // Tap gerçek-zamanlı ses iş parçacığında çalışır: MainActor'a dokunan
        // hiçbir şey yakalanmaz, yalnız Sendable yerel değerler.
        input.installTap(onBus: 0, bufferSize: 4096, format: girdiBicimi) { buffer, _ in
            guard let ready = donusturucu.transform(buffer) else { return }
            uc.yield(AnalyzerInput(buffer: ready))
        }

        engine.prepare()
        do {
            try engine.start()
        } catch {
            input.removeTap(onBus: 0)
            try? session.setActive(false, options: .notifyOthersOnDeactivation)
            throw VoiceError.hazirlanamadi
        }
        self.engine = engine
    }
}

/// Mikrofonun doğal biçimini çözümleyicinin istediği biçime çevirir.
/// Gerçek-zamanlı ses iş parçacığından çağrıldığı için bilinçli olarak
/// yalıtımsızdır; örneği yalnız tek bir tap kapanışı kullanır.
private nonisolated final class SpeechTranscriber: @unchecked Sendable {
    private let target: AVAudioFormat
    private var donusturucu: AVAudioConverter?

    init(target: AVAudioFormat) {
        self.target = target
    }

    func transform(_ buffer: AVAudioPCMBuffer) -> AVAudioPCMBuffer? {
        let source = buffer.format
        if source == target { return buffer }

        if donusturucu == nil || donusturucu?.inputFormat != source {
            let new = AVAudioConverter(from: source, to: target)
            // Ön-örnekleme kapalı: ilk örneklerin kalitesinden feragat edip
            // canlı akışta zaman damgası kaymasını önleriz.
            new?.primeMethod = AVAudioConverterPrimeMethod.none
            donusturucu = new
        }
        guard let donusturucu else { return nil }

        let ratio = target.sampleRate / source.sampleRate
        let kapasite = AVAudioFrameCount((Double(buffer.frameLength) * ratio).rounded(.up))
        guard kapasite > 0, let output = AVAudioPCMBuffer(pcmFormat: target, frameCapacity: kapasite) else { return nil }

        var verildi = false
        var error: NSError?
        donusturucu.convert(to: output, error: &error) { _, state in
            // Tek tamponluk dönüşüm: ikinci istekte "şimdilik veri yok" deriz.
            if verildi {
                state.pointee = .noDataNow
                return nil
            }
            verildi = true
            state.pointee = .haveData
            return buffer
        }
        guard error == nil, output.frameLength > 0 else { return nil }
        return output
    }
}
