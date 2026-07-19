//
//  NobetPanosu.swift
//  ketum
//
//  Nöbet panosu — zamanlanmış ajanların listesi. Her nöbet hairline çerçeveli
//  bir kart: adı, özeti ve son çalışma bilgisi. Kart detaya (çalışma günlüğüne)
//  götürür. Listenin sonunda kesikli çerçeveli "Yeni nöbet" satırı durur.
//

import SwiftUI
import SwiftData

struct NobetPanosu: View {
    let servis: ModelServisi

    @Query(sort: \Nobet.olusturulma, order: .reverse) private var nobetler: [Nobet]
    @Environment(\.modelContext) private var kayit
    @Environment(\.dismiss) private var kapat

    @State private var yol: [Nobet] = []
    @State private var kurulum = false
    /// Yazma hatasının kullanıcıya görünen karşılığı.
    @State private var uyariMetni: String?

    var body: some View {
        NavigationStack(path: $yol) {
            liste
                .background(Renk.zemin)
                .navigationTitle("Nöbetler")
                .navigationBarTitleDisplayMode(.inline)
                .navigationDestination(for: Nobet.self) { nobet in
                    NobetDetayi(nobet: nobet,
                                servis: servis.nobetBaglami.servis,
                                silmeIstegi: { silmeyeGonder(nobet) })
                }
                .toolbar {
                    ToolbarItem(placement: .topBarLeading) {
                        Button("Kapat") { kapat() }
                            .font(Yazi.cip())
                            .foregroundStyle(Renk.gri)
                    }
                }
                .sheet(isPresented: $kurulum) {
                    YeniNobet(baglam: servis.nobetBaglami)
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
                if nobetler.isEmpty {
                    bosDurum
                } else {
                    Text("sirr'in senin yerine periyodik yaptığı işler.")
                        .font(Yazi.cip())
                        .foregroundStyle(Renk.gri)
                        .padding(.bottom, Olcek.s1)
                        .satirZemini()
                }
            }

            ForEach(nobetler) { nobet in
                NavigationLink(value: nobet) {
                    satir(nobet)
                }
                .satirZemini()
            }
            .onDelete { indeksler in
                // Önce nesneleri topla: canlı @Query üzerinde indeksle silmek kayma üretir.
                for nobet in indeksler.map({ nobetler[$0] }) { sil(nobet) }
            }

            Section {
                yeniNobetSatiri
                    .satirZemini()
            }
        }
        .listStyle(.plain)
        .scrollContentBackground(.hidden)
    }

    private func satir(_ nobet: Nobet) -> some View {
        VStack(alignment: .leading, spacing: Olcek.s1) {
            HStack(spacing: Olcek.s2) {
                Text(nobet.ad)
                    .font(Yazi.kullanici())
                    .foregroundStyle(Renk.murekkep)
                    .lineLimit(1)
                if !nobet.aktif {
                    Text("duraklatıldı")
                        .font(Yazi.cip())
                        .foregroundStyle(Renk.soluk)
                }
            }
            Text(nobet.ozet)
                .font(Yazi.cip())
                .foregroundStyle(Renk.gri)
                .lineLimit(1)
            HStack(spacing: Olcek.s1) {
                Text(sonCalisma(nobet))
                    .font(Yazi.cip())
                    .foregroundStyle(Renk.soluk)
                    .lineLimit(1)
                if nobet.sonKayit?.basarili == true { OnayImi() }
            }
        }
        .padding(.vertical, Olcek.s3)
        .padding(.horizontal, Olcek.s4)
        .frame(maxWidth: .infinity, alignment: .leading)
        .overlay(
            RoundedRectangle(cornerRadius: Olcek.s4, style: .continuous)
                .stroke(Renk.cizgi, lineWidth: Olcek.hairline)
        )
    }

    private var yeniNobetSatiri: some View {
        Button { kurulum = true } label: {
            HStack(spacing: Olcek.s2) {
                Image(systemName: "plus")
                Text("Yeni nöbet")
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
    }

    private var bosDurum: some View {
        VStack(alignment: .leading, spacing: Olcek.s2) {
            Text("Henüz nöbet yok.")
                .font(Yazi.ketum())
                .foregroundStyle(Renk.murekkep)
            Text("Aşağıdan kur ya da sohbette iste: sabahları brifing hazırlayayım.")
                .font(Yazi.cip())
                .foregroundStyle(Renk.gri)
        }
        .padding(.vertical, Olcek.s4)
        .satirZemini()
    }

    // "Son: bugün 02.41" / "Son: 18 Tem 02.15" / henüz çalışmadıysa dürüst bir satır.
    private func sonCalisma(_ nobet: Nobet) -> String {
        guard let son = nobet.sonKayit else {
            return String(localized: "Henüz çalışmadı")
        }
        let ne = NobetBicim.tarihSaat(son.tarih)
        return son.basarili
            ? String(localized: "Son: \(ne)")
            : String(localized: "Son: \(ne) · atlandı")
    }

    // MARK: - Silme

    /// Detaydan gelen silme isteği: önce yığından çık, sonra sil. Böylece
    /// pop animasyonu sırasında silinmiş model okunmaz.
    private func silmeyeGonder(_ nobet: Nobet) {
        yol.removeAll()
        Task { @MainActor in
            try? await Task.sleep(for: .milliseconds(400))
            sil(nobet)
        }
    }

    private func sil(_ nobet: Nobet) {
        guard !nobet.isDeleted else { return }
        servis.nobetBaglami.servis.bildirimIptal(nobet)
        kayit.delete(nobet)
        do {
            try kayit.save()
        } catch {
            // Silme diske işlenmedi. Geri alıp bildirimini de geri planlıyoruz:
            // aksi halde listede duran nöbet sessizce bildirimsiz kalırdı.
            kayit.rollback()
            if !nobet.isDeleted { servis.nobetBaglami.servis.bildirimPlanla(nobet) }
            uyariMetni = String(localized: "Nöbet silinemedi: \(error.localizedDescription) Nöbet duruyor.")
        }
    }
}

/// Başarılı çalışma imi — renksiz, mürekkep dilinde.
struct OnayImi: View {
    var body: some View {
        Image(systemName: "checkmark")
            .font(Yazi.cip())
            .foregroundStyle(Renk.gri)
            .accessibilityLabel(Text("başarılı"))
    }
}

private extension View {
    /// Panodaki her satırın ortak zemini: sistem ayraçları ve dolgusu yok.
    func satirZemini() -> some View {
        listRowBackground(Renk.zemin)
            .listRowSeparator(.hidden)
            .listRowInsets(EdgeInsets(top: Olcek.s1, leading: Olcek.s5,
                                      bottom: Olcek.s1, trailing: Olcek.s5))
    }
}

/// Nöbet yüzeylerinde ortak tarih biçimleri.
enum NobetBicim {
    /// "bugün 02.41" / "dün 08.12" / "18 Tem 02.15"
    static func tarihSaat(_ d: Date) -> String {
        let takvim = Calendar.current
        let s = saat(d)
        if takvim.isDateInToday(d) { return String(localized: "bugün \(s)") }
        if takvim.isDateInYesterday(d) { return String(localized: "dün \(s)") }
        let f = DateFormatter(); f.locale = .current; f.dateFormat = "d MMM"
        return "\(f.string(from: d)) \(s)"
    }

    static func saat(_ d: Date) -> String {
        let f = DateFormatter(); f.locale = .current; f.dateFormat = "HH.mm"
        return f.string(from: d)
    }
}
