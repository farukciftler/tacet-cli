//
//  AracYurutucu.swift
//  Tacet
//
//  Araç çipi ↔ tool eşlemesi (spec §7.4). Tek doğruluk kaynağı: aracın kendisi.
//  Tool çağrısı başlayınca "çalışıyor" çipi düşer; tool dönünce çip son
//  durumuna geçer. Çip metni araç tarafından üretilir, modele yazdırılmaz.
//

import Foundation
import Observation

/// Bir onay isteğinin sonucu. `Bool` YETMEZDİ: "kullanıcı reddetti" ile
/// "kullanıcıya HİÇ sorulamadı" aynı `false`a düşüyor, araç ikisini de ret
/// diye modele raporluyordu — model de "paylaşmayı reddettiniz" diyordu.
/// Oysa çakışma yolunda kullanıcı o soruyu hiç görmemiştir; söylenen şey
/// kullanıcının yaptığı bir şey hakkında YALANDIR.
///
/// Tip DOSYA DÜZEYİNDEDİR, `AracYurutucu` içinde değil: `OnayKapisi` sözleşmesi
/// (Araclar/MCPAraci.swift) bunu döndürür ve o protokolün amacı araçları somut
/// yürütücüden bağımsız tutmaktır.
enum ApprovalDecision: Equatable {
    /// Onaylandı — ya da oturum temiz olduğu için hiç sorulmadı.
    case accepted
    /// Kullanıcı "gönderme" dedi (ya da bu kaynak bu oturumda zaten reddedilmişti).
    case denied
    /// Aynı anda başka bir onay bekleniyordu; bu istek kullanıcıya hiç
    /// gösterilmedi. Ret DEĞİLDİR: sonraki turda yeniden denenebilir.
    case busy
    /// Tur iptal edildi ("dur") ya da yeni tur başladı; karar hiç alınmadı.
    case cancelled

    /// Modele dönen İngilizce OLGU cümlesi; `nil` = kapı açık, akış continuation eder.
    ///
    /// Dördü de AYRI cümledir: "reddetti" kullanıcının yaptığı bir şeydir,
    /// diğer ikisi değildir. Model bunları ayırt edemezse kullanıcıya onun
    /// hiç vermediği bir karar atfeder.
    var toModel: String? {
        switch self {
        case .accepted:
            nil
        case .denied:
            "user_declined: the user chose not to send this data; nothing was sent"
        case .busy:
            "approval_unavailable: another approval was already pending, so the user "
                + "was never asked; nothing was sent"
        case .cancelled:
            "turn_cancelled: the turn was stopped before the user answered; nothing was sent"
        }
    }
}

/// Araçların çip yaşam döngüsünü bildirdiği arayüz. MainActor — UI durumu.
@MainActor
protocol ToolReporter: AnyObject, Sendable {
    /// "Çalışıyor" çipi düşürür, çip kimliğini döndürür.
    func start(icon: String, text: String) -> UUID
    /// Çipi son durumuna geçirir. Verilmeyen alanlar korunur.
    func update(_ id: UUID, state: ToolState, text: String?, rawInput: String?, rawOutput: String?, filePath: String?)
}

/// Aktif turdaki araç çiplerini biriktiren yürütücü. ModelServisi sahibidir;
/// SohbetGorunumu canlı çipleri buradan gözlemler, tur bitince Mesaj'a taşınır.
@MainActor
@Observable
final class ToolExecutor: ToolReporter {
    /// Aktif turda düşen çipler, çağrı sırasına göre.
    private(set) var traces: [ToolTrace] = []

    /// Bu turda "dünyayı değiştiren" (`.yazildi`) bir araç çalıştı mı — etkinlik
    /// yazıldı, hatırlatıcı kuruldu, belge üretildi/düzenlendi.
    ///
    /// YAPIŞKAN: hata kurtarma yolu retry'dan önce `newTurn(yanEtkiyiUnut: false)`
    /// çağırıyor; bayrak orada sıfırlansaydı yan etkinin olup olmadığı bilgisi
    /// tam da ihtiyaç duyulduğu anda kaybolurdu. Yalnızca gerçek yeni tur
    /// (`newTurn(yanEtkiyiUnut:)` varsayılanı) sıfırlar.
    private(set) var dunyaDegisti = false

    /// Bu turda UZAK (cihaz dışı) bir çağrı başarıyla döndü mü — yani
    /// kullanıcının kendi sunucusunda bir yan etki oluşmuş OLABİLİR.
    ///
    /// `dunyaDegisti`den AYRI bir eksendir ve ayrı olmak zorundadır: o bayrak
    /// yalnızca `.yazildi` çipiyle kurulur, uzak çağrı ise bilerek `.okundu`
    /// bırakılıyor (`MCPAraci.uzagaCagir`: `.yazildi` yerel geri alma/kurtarma
    /// mantığını tetikler ve uzak çağrı için yanlış olur). Sonuç: Jira issue'su
    /// açan bir çağrıdan SONRA oluşan genel bir hata `hataKurtar`ı retry'a
    /// sokuyor, AYNI istem ikinci kez gidiyor ve ikinci issue açılıyordu.
    ///
    /// `dunyaDegisti` gibi YAPIŞKAN: kurtarma yolu `newTurn(yanEtkiyiUnut: false)`
    /// çağırdığında korunur, yalnız gerçek yeni tur sıfırlar.
    ///
    /// SINIF AYRIMI YAPILMAZ (salt-okuma/yazma). Sınıflandırma (`YanEtkiSinifi`)
    /// sunucunun ipuçlarına ve ad sezgisine dayanan bir TAHMİNDİR; "salt-okuma"
    /// sanılan bir uzak aracın kayıt tuttuğu, sayaç artırdığı, e-posta
    /// tetiklediği durumda geri alınamaz. Bir retry'ı fazladan engellemenin
    /// maliyeti bir hata balonu; kaçırmanın maliyeti çift dış yan etki.
    private(set) var disEtkiOlusabilir = false

    /// Uzak çağrı BAŞARIYLA döndükten sonra çağrılır. Tek yön; tur boyunca kalır.
    func disEtkiIsaretle() { disEtkiOlusabilir = true }

    /// Retry güvenli mi — `hataKurtar` tek karar noktası olarak bunu okur.
    /// İki eksen de temizse aynı istem ikinci kez gönderilebilir.
    var retryGuvenli: Bool { !dunyaDegisti && !disEtkiOlusabilir }

    // MARK: - Kirli oturum (mcp §5.6)

    /// Bu oturumda kişisel veri aracı (Takvim, Kişi, Arama/Spotlight, Belge*,
    /// Hatırlatıcı) en az bir kez BAŞARIYLA çalıştı mı.
    ///
    /// Oturum boyunca TEMİZLENMEZ ve `newTurn()` bunu sıfırlamaz: bağlam
    /// özetlemesiyle yeni bir session açıldığında özetin kendisi kişisel veri
    /// taşıyabilir, dolayısıyla kirlilik de taşınır. Yalnızca gerçek sohbet
    /// sıfırlaması (`sohbetiSifirla()`) temizler.
    ///
    /// Modelin bu bayrağa erişimi yoktur; kapı kodda, modelde değil (mcp §2.2).
    private(set) var sessionTainted = false

    /// Kişisel veri aracının başarılı çağrısından sonra çağrılır. Tek yön.
    func taint() { sessionTainted = true }

    /// Bu oturumda kullanıcının paylaşımı reddettiği kaynaklar. Aynı kaynak
    /// için ikinci onay çipi üretilmez; sonraki denemeler sessizce aynı reddi
    /// alır — model ısrar döngüsüne giremez (mcp §3.3).
    private var reddedilenKaynaklar: Set<String> = []

    /// Şu an kullanıcı kararı bekleyen istek. Görünüm katmanı bunu gözler ve
    /// `OnaySayfasi`nı sunar; karar `onayKarariVer(_:)` ile geri gelir.
    private(set) var pendingApproval: ApprovalRequest?

    /// Kararı bekleyen `onayIste` çağrısının devamı. Aynı anda en fazla bir tane.
    private var onayDevami: CheckedContinuation<ApprovalDecision, Never>?

    /// Kullanıcıya sorulacak tek bir paylaşım isteği.
    struct ApprovalRequest: Identifiable, Equatable {
        let id = UUID()
        /// Bağlantı/sunucu adı — "ev sunucusu", "arama sunucusu".
        let source: String
        let toolName: String
        /// GÖNDERİLECEK ARGÜMANLARIN AYNISI. Kategori özeti değil.
        let content: String
        /// Bu isteğin akıştaki çipi.
        let traceID: UUID
    }

    /// Cihaz dışına veri çıkarmadan önce çağrılır. `.kabul` dışında hiçbir şey
    /// gönderilmez.
    ///
    /// - Oturum kirli değilse SORMADAN `.kabul` döner: onay nadirse okunur (mcp §2.4).
    /// - Kaynak bu oturumda bir kez reddedildiyse çip düşmeden `.reddedildi` döner.
    /// - Başka bir kapı açıksa `.mesgul` — bu RET DEĞİLDİR, soru hiç sorulmadı.
    /// - Aksi halde akışa "onay bekleniyor" çipi düşer ve kullanıcı kararı beklenir.
    ///
    /// - Parameter icerik: gönderilecek argümanların aynısı; kullanıcı onay
    ///   sayfasında birebir bunu görür.
    ///
    /// Onay kapısının TAM sonucu — TEK giriş noktası. `Bool` döndüren kısa
    /// biçimler kaldırıldı: orada ret, çakışma ve iptal aynı `false`a düşüyordu
    /// ve yeni bir çağıranın bu ayrımı sessizce kaybetmesi an meselesiydi.
    ///
    /// - Parameter zorunlu: true ise oturum temiz olsa da sorulur. Yıkıcı uzak
    ///   araçlar bunu kullanır: orada soru "cihazdan ne çıkıyor" değil,
    ///   "kullanıcının sunucusunda ne değişiyor" — kirlilikle ilgisiz.
    func requestApprovalDecision(source: String,
                          toolName: String,
                          content: String,
                          required: Bool = false) async -> ApprovalDecision {
        guard sessionTainted || required else { return .accepted }
        if reddedilenKaynaklar.contains(source) { return .denied }
        // Aynı anda ikinci bir kapı açmayız — kullanıcı iki sheet arasında
        // sıkışmaz. Ama bu RET DEĞİLDİR: soru hiç sorulmadı, çip de düşmez.
        if pendingApproval != nil { return .busy }

        let traceID = start(icon: "hand.raised", text: L10n.connectionAwaitingApproval(source))
        update(traceID, state: .awaitingApproval, rawInput: content)

        let decision: ApprovalDecision = await withCheckedContinuation { continuation in
            onayDevami = continuation
            pendingApproval = ApprovalRequest(source: source, toolName: toolName,
                                      content: content, traceID: traceID)
        }

        switch decision {
        case .accepted:
            // Çip normal yaşam döngüsüne döner: aracın kendi çipi devralır,
            // bekleme çipi akışta iz bırakmaz.
            traces.removeAll { $0.id == traceID }
        case .denied:
            reddedilenKaynaklar.insert(source)
            update(traceID, state: .notSent, text: L10n.connectionNotSent(source))
        case .cancelled, .busy:
            // İPTAL RET DEĞİLDİR: kaynak oturum boyunca kara listeye ALINMAZ.
            // Kullanıcı "dur"a bastı diye bir sunucuya bir daha hiç veri
            // gönderememek, onun hiç vermediği bir karara kalıcılık yüklemek
            // olurdu. Çip yine de "gönderilmedi" kalır — hiçbir şey çıkmadı.
            update(traceID, state: .notSent, text: L10n.connectionNotSent(source))
        }
        return decision
    }

    /// Kullanıcının onay sayfasındaki kararı. Sayfayı kapatmak = `false`.
    func decideApproval(_ accepted: Bool) {
        onayiCoz(accepted ? .accepted : .denied)
    }

    /// Bekleyen bir onay varsa İPTAL olarak çözer — turun iptali askıda bir
    /// continuation bırakmasın (aksi halde araç görevi bir sonraki mesaja kadar
    /// askıda sızar ve onay sayfası ekranda kalır).
    ///
    /// `newTurn()` ve `ModelServisi.durdur()` çağırır; ikisi de dışarıdan
    /// erişilebilmeli.
    func bekleyenOnayiCoz() { onayiCoz(.cancelled) }

    /// Continuation TEK KEZ resume edilir: alan resume'dan ÖNCE nil'lenir,
    /// ikinci çağrı `guard`da düşer. (Çift resume çalışma anı çökmesidir ve
    /// `durdur()` ile kullanıcının onay sayfasındaki kararı yarışabilir.)
    private func onayiCoz(_ decision: ApprovalDecision) {
        guard let continuation = onayDevami else { return }
        onayDevami = nil
        pendingApproval = nil
        continuation.resume(returning: decision)
    }

    // MARK: - Tur yaşam döngüsü

    /// Tur başına sıfırlanan dış sayaçların kancası (kod-spec §5.4: kod deneme
    /// sayacı `AracYurutucu.newTurn`'da sıfırlanır). Sahibi (ModelServisi)
    /// kurar; jenerik tutulur — bu sınıf sayaç türlerini tanımaz. Kanca TEK
    /// noktadan tetiklendiği için `newTurn` çağıran gelecekteki bir yol
    /// sayaç sıfırlamayı unutamaz.
    var turKancasi: (() -> Void)?

    /// Yeni tur — önceki turun çipleri Mesaj'a taşındıktan sonra sıfırlanır.
    ///
    /// `yanEtkiyiUnut: false` = "bu gerçek bir yeni tur DEĞİL, aynı turun
    /// kurtarma denemesi". O durumda ne yan etki bayrakları ne de ÇİPLER
    /// sıfırlanır (denetim P2-8).
    ///
    /// Çiplerin de korunması bilinçli bir geri dönüştür: eski davranış
    /// `izler = []`i koşulsuz yapıyordu ve kurtarma sonrası kullanıcı ilk
    /// denemede hangi aracın çalıştığını göremiyordu — tam da hata teşhisinin
    /// en çok ihtiyaç duyduğu bilgi, hata anında siliniyordu. İki deneme aynı
    /// aracı çalıştırırsa iki çip görünür; bu bir kusur değil, olan bitenin
    /// kendisidir ("bir kez denendi, hata aldı, yeniden denendi").
    ///
    /// Kirli oturum bayrağını ve ret önbelleğini BİLEREK sıfırlamaz — ikisi de
    /// oturum ömürlüdür (mcp §5.6, §3.3).
    func newTurn(yanEtkiyiUnut: Bool = true) {
        bekleyenOnayiCoz()
        if yanEtkiyiUnut {
            traces = []
            dunyaDegisti = false
            disEtkiOlusabilir = false
        }
        turKancasi?()
    }

    /// Gerçek sohbet sıfırlaması: oturum ömürlü ne varsa burada biter —
    /// kirlilik ve ret önbelleği dahil. Yeni sohbet temiz başlar.
    func resetChat() {
        newTurn()
        sessionTainted = false
        reddedilenKaynaklar.removeAll()
    }

    func start(icon: String, text: String) -> UUID {
        let trace = ToolTrace(icon: icon, text: text, state: .running)
        traces.append(trace)
        return trace.id
    }

    func update(_ id: UUID,
                  state: ToolState,
                  text: String? = nil,
                  rawInput: String? = nil,
                  rawOutput: String? = nil,
                  filePath: String? = nil) {
        if state == .written { dunyaDegisti = true }
        guard let i = traces.firstIndex(where: { $0.id == id }) else { return }
        traces[i].state = state
        if let text { traces[i].text = text }
        if let rawInput { traces[i].rawInput = rawInput }
        if let rawOutput { traces[i].rawOutput = rawOutput }
        if let filePath { traces[i].filePath = filePath }
    }
}
