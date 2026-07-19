//
//  BaglantiPanosu.swift
//  ketum
//
//  Bağlantılar (MCP) panosu — mcp-baglanti-spec §4, §3.5.
//
//  Boş durum vaadi olduğu gibi söyler: bağlanmadıkça sirr internete çıkmaz.
//  Satır: ad, URL, araç sayısı, son kullanım. Silme onaylıdır ve sonucunu
//  söyler.
//

import SwiftUI
import SwiftData

// MARK: - Servis sözleşmesi

/// "Bağlantıyı dene" ve yaşam döngüsü — `BaglantiServisi` uygular.
/// Görünüm katmanı ağ API'sine dokunmaz, yalnızca bunu çağırır (§2.1).
@MainActor
protocol BaglantiDeneyici: AnyObject {
    /// Henüz kaydedilmemiş form için `initialize` + `tools/list`.
    /// Ekleme öncesi zorunlu adım (§3.1).
    func dene(ad: String, urlHam: String, anahtar: String?) async -> BaglantiDenemeSonucu
    /// Var olan bağlantıyı yeniden dener; araç özetlerini tazeler (§5.3).
    func dene(_ baglanti: Baglanti) async -> BaglantiDenemeSonucu
    /// Anahtarı Keychain'e yazar, bağlantıyı kalıcılaştırır.
    func ekle(ad: String,
              urlHam: String,
              anahtar: String?,
              cihazVerisi: CihazVerisiAyari,
              araclar: [AracOzeti]) throws -> Baglanti
    /// Bağlantı silinmeden ÖNCE çağrılır: Keychain kaydını kaldırır.
    func anahtariKaldir(_ baglanti: Baglanti)
}

/// Deneme sonucu. Başarısızlıkta neden düz dille yazılır, sessizce yutulmaz.
enum BaglantiDenemeSonucu {
    /// Sunucunun döndürdüğü araçlar — desteklenmeyenler de listede.
    case basarili([AracOzeti])
    /// Zaman aşımı / yetki / TLS gibi nedenin düz dilli karşılığı.
    case basarisiz(String)
}

// MARK: - Pano

struct BaglantiPanosu: View {
    let servis: any BaglantiDeneyici

    @Query(sort: \Baglanti.olusturulma, order: .reverse) private var baglantilar: [Baglanti]
    @Environment(\.modelContext) private var kayit
    @Environment(\.dismiss) private var kapat

    @State private var yol: [Baglanti] = []
    @State private var kurulum = false
    @State private var uyariMetni: String?

    var body: some View {
        NavigationStack(path: $yol) {
            liste
                .background(Renk.zemin)
                .navigationTitle("Bağlantılar")
                .navigationBarTitleDisplayMode(.inline)
                .navigationDestination(for: Baglanti.self) { baglanti in
                    BaglantiDetayi(baglanti: baglanti,
                                   servis: servis,
                                   silmeIstegi: { silmeyeGonder(baglanti) })
                }
                .toolbar {
                    ToolbarItem(placement: .topBarLeading) {
                        Button("Kapat") { kapat() }
                            .font(Yazi.cip())
                            .foregroundStyle(Renk.gri)
                    }
                }
                .sheet(isPresented: $kurulum) {
                    YeniBaglanti(servis: servis)
                }
                .alert(Text("Bir sorun oldu"), isPresented: Binding(
                    get: { uyariMetni != nil },
                    set: { if !$0 { uyariMetni = nil } }
                )) {
                    Button(role: .cancel) { uyariMetni = nil } label: { Text("Tamam") }
                } message: {
                    if let uyariMetni { Text(uyariMetni) }
                }
        }
    }

    // MARK: - Liste

    private var liste: some View {
        List {
            Section {
                if baglantilar.isEmpty {
                    bosDurum
                } else {
                    Text("sirr yalnızca burada bağladığın sunuculara çıkar.")
                        .font(Yazi.cip())
                        .foregroundStyle(Renk.gri)
                        .padding(.bottom, Olcek.s1)
                        .baglantiSatiri()
                }
            }

            ForEach(baglantilar) { baglanti in
                NavigationLink(value: baglanti) {
                    satir(baglanti)
                }
                .baglantiSatiri()
            }

            Section {
                sunucuEkleSatiri
                    .baglantiSatiri()
            }
        }
        .listStyle(.plain)
        .scrollContentBackground(.hidden)
    }

    private func satir(_ baglanti: Baglanti) -> some View {
        VStack(alignment: .leading, spacing: Olcek.s1) {
            Text(baglanti.ad)
                .font(Yazi.kullanici())
                .foregroundStyle(Renk.murekkep)
                .lineLimit(1)
            Text(baglanti.urlHam)
                .font(Yazi.cip())
                .foregroundStyle(Renk.gri)
                .lineLimit(1)
                .truncationMode(.middle)
            Text(altSatir(baglanti))
                .font(Yazi.cip())
                .foregroundStyle(Renk.soluk)
                .lineLimit(1)
        }
        .padding(.vertical, Olcek.s3)
        .padding(.horizontal, Olcek.s4)
        .frame(maxWidth: .infinity, alignment: .leading)
        .overlay(
            RoundedRectangle(cornerRadius: Olcek.s4, style: .continuous)
                .stroke(Renk.cizgi, lineWidth: Olcek.hairline)
        )
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(Text(verbatim: "\(baglanti.ad). \(baglanti.urlHam)"))
        .accessibilityValue(Text(verbatim: altSatir(baglanti)))
    }

    /// "4 araç · son kullanım bugün 09.12" — hiç kullanılmadıysa öyle der.
    private func altSatir(_ baglanti: Baglanti) -> String {
        let sayi = baglanti.kullanilabilirAraclar.count
        let araclar = String(localized: "\(sayi) araç")
        guard let son = baglanti.sonKullanim else {
            return araclar + " · " + String(localized: "henüz kullanılmadı")
        }
        return araclar + " · " + String(localized: "son kullanım \(NobetBicim.tarihSaat(son))")
    }

    private var sunucuEkleSatiri: some View {
        Button { kurulum = true } label: {
            HStack(spacing: Olcek.s2) {
                Image(systemName: "plus")
                    .accessibilityHidden(true)
                Text("Sunucu ekle")
                Spacer(minLength: 0)
            }
            .font(Yazi.kullanici())
            .foregroundStyle(Renk.gri)
            .padding(.vertical, Olcek.s3)
            .padding(.horizontal, Olcek.s4)
            .overlay(
                RoundedRectangle(cornerRadius: Olcek.s4, style: .continuous)
                    .stroke(Renk.cizgi, style: StrokeStyle(lineWidth: Olcek.hairline, dash: [4, 4]))
            )
        }
        .buttonStyle(.plain)
        .accessibilityHint(Text("Kendi MCP sunucunu eklemek için form açar."))
    }

    // Metin §4'te birebir yazılıdır.
    private var bosDurum: some View {
        VStack(alignment: .leading, spacing: Olcek.s2) {
            Text("Bağlı sunucu yok.")
                .font(Yazi.ketum())
                .foregroundStyle(Renk.murekkep)
            Text("Kendi MCP sunucunu bağlarsan sirr oradaki araçları kullanabilir. Bağlanmadıkça sirr internete çıkmaz.")
                .font(Yazi.cip())
                .foregroundStyle(Renk.gri)
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(.vertical, Olcek.s4)
        .baglantiSatiri()
    }

    // MARK: - Silme

    /// Detaydan gelen silme isteği: önce yığından çık, sonra sil. Pop
    /// animasyonu sırasında silinmiş model okunmaz.
    private func silmeyeGonder(_ baglanti: Baglanti) {
        yol.removeAll()
        Task { @MainActor in
            try? await Task.sleep(for: .milliseconds(400))
            sil(baglanti)
        }
    }

    private func sil(_ baglanti: Baglanti) {
        guard !baglanti.isDeleted else { return }
        // Silinmiş nesnenin alanına dokunmak ölümcüldür: ad ve anahtar
        // temizliği delete'ten ÖNCE hallolur.
        let ad = baglanti.ad
        servis.anahtariKaldir(baglanti)
        kayit.delete(baglanti)
        do {
            try kayit.save()
        } catch {
            kayit.rollback()
            uyariMetni = String(localized: "\(ad) silinemedi: \(error.localizedDescription)")
        }
    }
}

private extension View {
    /// Panodaki her satırın ortak zemini: sistem ayraçları ve dolgusu yok.
    func baglantiSatiri() -> some View {
        listRowBackground(Renk.zemin)
            .listRowSeparator(.hidden)
            .listRowInsets(EdgeInsets(top: Olcek.s1, leading: Olcek.s5,
                                      bottom: Olcek.s1, trailing: Olcek.s5))
    }
}
