//
//  SohbetListesi.swift
//  ketum
//
//  Sohbet geçmişi (spec §4.7). Eski sohbetlere erişim, yeni sohbet, silme.
//  Sade liste: başlık + son satır önizlemesi + tarih. Süs yok.
//

import SwiftUI
import SwiftData

struct SohbetListesi: View {
    let sohbetler: [Sohbet]
    let aktifID: UUID?
    let sec: (Sohbet) -> Void
    let sil: (Sohbet) -> Void
    let yeni: () -> Void
    var nobetlerAc: () -> Void = {}
    var becerilerAc: () -> Void = {}
    var hafizaAc: () -> Void = {}
    var ayarlarAc: () -> Void = {}

    @Environment(\.dismiss) private var kapat

    @State private var arama = ""
    /// Silme onayı bekleyen sohbet. Onaydan önce hiçbir şey silinmez.
    @State private var silinecek: Sohbet?
    /// Nokta ve ikon sütunu metinle birlikte büyür; hizalama Dynamic Type'ta bozulmasın.
    @ScaledMetric(relativeTo: .callout) private var noktaBoyu = Olcek.nokta
    @ScaledMetric(relativeTo: .callout) private var ikonSutunu = Olcek.ikonSutunu

    /// Başlık ve mesaj içeriğinde arar. Arama boşken liste olduğu gibi kalır.
    private var suzulmus: [Sohbet] {
        let terim = arama.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !terim.isEmpty else { return sohbetler }
        return sohbetler.filter { sohbet in
            if sohbet.baslik.localizedCaseInsensitiveContains(terim) { return true }
            return sohbet.mesajlar.contains { $0.icerik.localizedCaseInsensitiveContains(terim) }
        }
    }

    var body: some View {
        NavigationStack {
            List {
                // Arama yapılırken menü satırları yolu tıkamasın.
                if arama.isEmpty {
                    menuSatiri(ikon: "moon.stars", baslik: "Nöbetler", eylem: nobetlerAc)
                    menuSatiri(ikon: "wand.and.stars", baslik: "Beceriler", eylem: becerilerAc)
                    menuSatiri(ikon: "text.book.closed", baslik: "Hafıza", eylem: hafizaAc)
                    menuSatiri(ikon: "gearshape", baslik: "Ayarlar", eylem: ayarlarAc)
                }

                if !arama.isEmpty && suzulmus.isEmpty {
                    Text("Eşleşen sohbet yok.")
                        .font(Yazi.cip())
                        .foregroundStyle(Renk.gri)
                        .listRowBackground(Renk.zemin)
                }

                ForEach(suzulmus) { sohbet in
                    Button {
                        sec(sohbet)
                    } label: {
                        satir(sohbet)
                    }
                    .buttonStyle(.plain)
                    .listRowBackground(Renk.zemin)
                }
                .onDelete { indeksler in
                    // Önce nesneyi al: canlı diziye silme anında indeksle dönmek kayma üretir.
                    silinecek = indeksler.map { suzulmus[$0] }.first
                }
            }
            .listStyle(.plain)
            .background(Renk.zemin)
            .searchable(text: $arama, prompt: Text("Sohbetlerde ara"))
            .confirmationDialog("Bu sohbet silinsin mi?",
                                isPresented: Binding(get: { silinecek != nil },
                                                     set: { if !$0 { silinecek = nil } }),
                                titleVisibility: .visible,
                                presenting: silinecek) { sohbet in
                Button("Sil", role: .destructive) {
                    // Silmeden ÖNCE dışarı ver; sonrasında nesnenin alanına dokunulmaz.
                    silinecek = nil
                    sil(sohbet)
                }
                Button("Vazgeç", role: .cancel) { silinecek = nil }
            } message: { sohbet in
                Text("“\(Self.gorunenAd(sohbet))” ve içindeki mesajlar silinir. Bu işlem geri alınamaz.")
            }
            .navigationTitle("Sohbetler")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button("Kapat") { kapat() }
                        .font(Yazi.cip())
                        .foregroundStyle(Renk.gri)
                }
                ToolbarItem(placement: .topBarTrailing) {
                    Button(action: yeni) {
                        Image(systemName: "square.and.pencil")
                            .foregroundStyle(Renk.murekkep)
                    }
                    .accessibilityLabel("Yeni sohbet")
                }
            }
        }
    }

    /// Listenin üstündeki üç yönlendirme satırı — tek gövde, tek erişilebilirlik düğmesi.
    private func menuSatiri(ikon: String,
                            baslik: LocalizedStringKey,
                            eylem: @escaping () -> Void) -> some View {
        Button(action: eylem) {
            HStack(spacing: Olcek.s3) {
                Image(systemName: ikon)
                    .font(Yazi.ikon())
                    .foregroundStyle(Renk.gri)
                    .frame(width: ikonSutunu)
                Text(baslik)
                    .font(Yazi.kullanici())
                    .foregroundStyle(Renk.murekkep)
                Spacer()
                Image(systemName: "chevron.right")
                    .font(Yazi.ikonKucuk())
                    .foregroundStyle(Renk.soluk)
            }
            .padding(.vertical, Olcek.s1)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .listRowBackground(Renk.zemin)
        // İkon ve chevron süs; okunan tek şey satırın adı.
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(Text(baslik))
        .accessibilityAddTraits(.isButton)
    }

    private func satir(_ sohbet: Sohbet) -> some View {
        let aktifMi = sohbet.id == aktifID
        let ad = Self.gorunenAd(sohbet)

        return HStack(spacing: Olcek.s3) {
            // Aktif sohbet için küçük mürekkep nokta (marka: renk değil, ağırlık/işaret).
            // Görsel gösterge; karşılığı aşağıda accessibilityValue ile söze dökülür.
            Circle()
                .fill(aktifMi ? Renk.murekkep : Color.clear)
                .frame(width: noktaBoyu, height: noktaBoyu)

            VStack(alignment: .leading, spacing: Olcek.s1) {
                Text(ad)
                    .font(Yazi.kullanici())
                    .foregroundStyle(Renk.murekkep)
                    .lineLimit(1)
                Text(sohbet.onizleme)
                    .font(Yazi.cip())
                    .foregroundStyle(Renk.gri)
                    .lineLimit(1)
            }
            Spacer()
            Text(tarih(sohbet.guncelleme))
                .font(Yazi.cip())
                .foregroundStyle(Renk.soluk)
        }
        .padding(.vertical, Olcek.s1)
        .contentShape(Rectangle())
        // Dört ayrı parça değil, tek sohbet satırı olarak okunur. Aktiflik noktası
        // yalnızca görsel kalmasın diye durum ayrıca söze dökülür.
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(Text(verbatim: "\(ad). \(sohbet.onizleme)"))
        .accessibilityValue(aktifMi
                            ? Text("Açık sohbet, \(tarih(sohbet.guncelleme))")
                            : Text(verbatim: tarih(sohbet.guncelleme)))
        .accessibilityHint(Text("Bu sohbeti açmak için çift dokun."))
        .accessibilityAddTraits(aktifMi ? [.isButton, .isSelected] : [.isButton])
    }

    /// Listede görünen sohbet adı. `isEmpty` kontrolü tek başına yetmiyordu:
    /// Sohbet'in varsayılan başlığı ham Türkçe "Yeni sohbet" literali, yani boş
    /// değil — İngilizce arayüzde de Türkçe görünüyordu. Başlık hâlâ otomatikse
    /// (kullanıcı ya da ilk mesaj belirlememişse) lokalize karşılığını gösteriyoruz.
    private static func gorunenAd(_ sohbet: Sohbet) -> String {
        let baslik = sohbet.baslik.trimmingCharacters(in: .whitespacesAndNewlines)
        if baslik.isEmpty || sohbet.baslikOtomatik { return String(localized: "Yeni sohbet") }
        return baslik
    }

    private func tarih(_ d: Date) -> String {
        let t = Calendar.current
        if t.isDateInToday(d) {
            let f = DateFormatter(); f.locale = Locale.current; f.dateFormat = "HH:mm"
            return f.string(from: d)
        }
        if t.isDateInYesterday(d) { return Yerel.dun }
        let f = DateFormatter(); f.locale = Locale.current; f.dateFormat = "d MMM"
        return f.string(from: d)
    }
}
