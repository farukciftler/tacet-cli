//
//  MCPAracKoprusu.swift
//  Tacet
//
//  MCP araç köprüsü (mcp §5.2, §5.4, §5.5). `ModelServisi.swift` içinden
//  OLDUĞU GİBİ taşındı — davranış değişikliği yoktur.
//

import Foundation
import FoundationModels

// MARK: - MCP araç köprüsü (mcp §5.2, §5.4, §5.5)

/// Kayıtlı bir bağlantıyı çalışır `MCPAraci` örneklerine çeviren ve uzak çağrıyı
/// yürüten katman. `MCPAraci` ağ API'sine dokunmaz, yalnızca `MCPCagirici`yi
/// çağırır; ağ hâlâ tek yerde (`MCPIstemcisi`).
///
/// İstemci bağlantı başına TEK örnektir: MCP oturum kimliği (`Mcp-Session-Id`)
/// istemcinin içinde yaşıyor, her çağrıda yeni istemci kurmak her çağrıda yeni
/// el sıkışma demek olurdu.
@MainActor
final class MCPToolBridge: MCPInvoker {

    private struct Endpoint: Equatable { let url: URL; let key: String? }

    private var ucNoktalar: [UUID: Endpoint] = [:]
    private var istemciler: [UUID: MCPClient] = [:]

    /// §5.5 sonuç işleme için — büyük çıktı modelden geçmeden buraya konur.
    weak var dataStore: DataStore?

    /// Dış yan etki bayrağının sahibi (denetim P0-3). Uzak çağrı sunucuya
    /// ULAŞTIĞI anda `disEtkiIsaretle()` çağrılır ve o turda retry kapanır.
    weak var executor: ToolExecutor?

    init() {}

    /// Bağlantının adresini kaydeder. Adres ya da anahtar değiştiyse istemci
    /// atılır: eski oturum kimliğiyle yeni sunucuya konuşulmaz.
    func save(identity: UUID, url: URL, key: String?) {
        let new = Endpoint(url: url, key: key)
        guard ucNoktalar[identity] != new else { return }
        ucNoktalar[identity] = new
        istemciler[identity] = nil
    }

    /// Tüm bağlantılar gitti (silindi / hiç yok): istemcileri bırak.
    func forget() {
        ucNoktalar.removeAll()
        istemciler.removeAll()
    }

    private func client(_ identity: UUID) -> MCPClient? {
        if let available = istemciler[identity] { return available }
        guard let uc = ucNoktalar[identity] else { return nil }
        let new = MCPClient(url: uc.url, key: uc.key)
        istemciler[identity] = new
        return new
    }

    // MARK: - Araç kurulumu

    /// Sunucudan şemaları okur ve oturuma girecek araçları üretir.
    ///
    /// Modele giden tanım SUNUCUNUN HAM AÇIKLAMASI DEĞİL, ekleme anında
    /// önbelleklenen özettir (§5.3) — ham açıklama 4096 pencereyi tek araçla
    /// doldurabilir. Önbellekte olmayan araç (yeni eklenmiş, henüz özetlenmemiş)
    /// bu turda atlanır; özet tazelenince gelir.
    ///
    /// Ağ erişilemezse boş dizi döner — bağlantı profili seçilemez, bugünkü
    /// davranış sürer. Uydurma araç üretilmez.
    func araclariKur(connectionID: UUID,
                     name: String,
                     ozetler: [ToolSummary],
                     pool: Int,
                     deviceData: DeviceDataSetting,
                     gate: (any ApprovalGate)?,
                     reporter: (any ToolReporter)?) async -> [MCPTool] {
        guard let client = client(connectionID), !ozetler.isEmpty else { return [] }
        guard let tanimlar = try? await client.tools() else { return [] }

        var ozetSozlugu: [String: String] = [:]
        for summary in ozetler where !summary.isUnsupported { ozetSozlugu[summary.name] = summary.summary }

        // Sunucu sırası korunur (deterministik), önbellekte olmayan elenir ve
        // HAVUZ tavanı ÇEVİRİDEN ÖNCE uygulanır: 200 araçlık sunucuda 200 şema
        // çevirmenin anlamı yok. Havuz, oturum yuvasından (6) BİLEREK geniştir:
        // yuvayı hangi araçların dolduracağına artık burada değil, kullanıcının
        // o turki isteğine bakan `AracAlaka` karar veriyor (P1-6) ve bunun için
        // seçebileceği bir havuz gerek. Havuz da 6 olsaydı alaka sıralaması
        // sunucunun ilk 6'sını kendi içinde yeniden dizmekten öteye gidemezdi.
        // Sınıflandırma tavandan ÖNCE: 200 araçlık bir sunucuda ilk 6 araç
        // `komut_calistir`, `dosya_sil` olabilir ve saf sunucu sırası bu
        // durumda oturumu yıkıcı araçlarla doldurup salt-okumaları dışarıda
        // bırakır. Kararlı sıralama (`enumerate` ile bağ bozma) sunucu sırasını
        // sınıf içinde korur, yani davranış hâlâ deterministik.
        let suzulmus = tanimlar.filter { ozetSozlugu[$0.name] != nil }
        let siniflar = suzulmus.map {
            SideEffectClass.classify(name: $0.name,
                                  summary: ozetSozlugu[$0.name] ?? "",
                                  readOnlyHint: $0.readOnlyHint,
                                  destructiveHint: $0.destructiveHint)
        }
        let adaylar = zip(suzulmus, siniflar).enumerated()
            .sorted { sol, sag in
                let solYikici = sol.element.1.requiresApproval
                let sagYikici = sag.element.1.requiresApproval
                if solYikici != sagYikici { return !solYikici }
                return sol.offset < sag.offset
            }
            .map(\.element)
            .prefix(pool)

        // Ad çakışması koleksiyon düzeyinde çözülür (P2-9). "get-user" ve
        // "get_user" ikisi de `get_user`a indirgeniyordu ve model iki araçtan
        // hangisini çağırdığını bilmiyordu; `adlariCoz` sırayı bozmadan
        // ikisine FARKLI ad verir.
        let cozulmusAdlar = MCPTool.resolveNames(adaylar.map { (remoteName: $0.0.name, server: name) })

        var tools: [MCPTool] = []
        for (order, (spec, sideEffect)) in adaylar.enumerated() {
            // Şema çalışma anında çevrilir; çevrilemeyen araç ATLANIR (§5.2) —
            // yanlış argüman üretmektense araç hiç olmasın.
            guard let schema = try? MCPSchemaConverter.convert(spec: Self.tanimaCevir(spec)) else { continue }
            tools.append(MCPTool(connectionID: connectionID,
                                    connectionName: name,
                                    remoteName: spec.name,
                                    summary: ozetSozlugu[spec.name] ?? "",
                                    parameters: schema,
                                    invoker: self,
                                    deviceData: deviceData,
                                    sideEffect: sideEffect,
                                    gate: gate,
                                    reporter: reporter,
                                    resolvedName: cozulmusAdlar[order]))
        }
        return tools
    }

    /// İstemci tanımı → şema çevirisinin beklediği ham tanım.
    /// Şemasız araç = argümansız araç: boş nesne şeması verilir, araç düşmez.
    private static func tanimaCevir(_ spec: MCPClient.ToolSpec) -> MCPToolSpec {
        let bosNesne = Data(#"{"type":"object","properties":{}}"#.utf8)
        var data = bosNesne
        if let schema = spec.schema, case .object = schema,
           let kodlanmis = try? JSONEncoder().encode(schema) {
            data = kodlanmis
        }
        return MCPToolSpec(name: spec.name, description: spec.description, inputSchemaJSON: data)
    }

    // MARK: - Uzak çağrı (MCPCagirici)

    /// Onay kapısı bu çağrıdan ÖNCE `MCPAraci.call` içinde geçildi; buraya gelen
    /// her şey kullanıcının gördüğü şeydir.
    func invoke(connectionID: UUID, toolName: String, argumentsJSON: String) async throws -> MCPOutcome {
        guard let client = client(connectionID) else {
            throw MCPClient.MCPError.unreachable
        }
        // Model şemaya uygun JSON üretir; yine de ayrıştırılamayan girdiyi
        // uydurmayız — argümansız çağrıya ineriz.
        let argumanlar = JSONValue.parse(argumentsJSON) ?? .object([:])
        let (text, hataliMi) = try await client.aracCagir(name: toolName, argumanlar: argumanlar)

        // BURASI P0-3'ÜN TEK NOKTASI. Çağrı DÖNDÜ demek, istek sunucuya ulaştı
        // ve sunucu onu işledi demektir — issue açıldı, kayıt yazıldı, e-posta
        // gitti olabilir. Bundan sonra bu turda AYNI istemi ikinci kez
        // göndermek geri alınamaz bir tekrar üretir.
        //
        // `hataliMi` AYIRMAZ ve ayırmamalı: MCP'de `isError` sunucunun aracın
        // sonucu hakkındaki yorumudur, işlemin hiç gerçekleşmediğinin kanıtı
        // değil ("issue açıldı ama alan doğrulaması geçmedi" de isError döner).
        // Yalnız `throw` eden yol (taşıma hatası) bayrağı kurmaz, çünkü orada
        // istek sunucuya ulaşmamıştır.
        executor?.disEtkiIsaretle()

        // §5.5: ham çıktı modele girmez; özet + kaynakRef gider, tamamı çipte kalır.
        let islenmis = ConnectionService.sonucIsle(text, toolName: toolName, dataStore: dataStore)
        let body = islenmis.toModel.trimmingCharacters(in: .whitespacesAndNewlines)

        if hataliMi {
            // Sunucunun KENDİ hatası (komut başarısız) — taşıma hatası değil.
            // Model bunu okuyup anlatır; çip de "hata döndü" der, sessiz geçilmez.
            return MCPOutcome(
                chipDetail: String(localized: "\(toolName) returned an error"),
                toModel: body.isEmpty
                    ? "remote_tool_error: the tool failed on the user's server without a message. Say this in one sentence."
                    : "remote_tool_error: \(body)",
                rawOutput: islenmis.rawOutput)
        }
        return MCPOutcome(
            chipDetail: String(localized: "\(toolName) done"),
            toModel: body.isEmpty
                ? "remote_tool_empty: the tool ran but returned nothing. Say this in one sentence; do not invent a result."
                : body,
            rawOutput: islenmis.rawOutput)
    }
}
