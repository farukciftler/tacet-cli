//
//  HafizaPanosu.swift
//  ketum
//
//  Hafıza panosu (hafiza-spec §6.1) — BeceriPanosu ile aynı iskelet.
//
//  Katmanın birinci ilkesi burada görünür hale gelir: "sessizce öğrenme yok".
//  Ayıklanan her not bu listede durur, düzenlenebilir ve silinebilir. Sohbet
//  içinde "not aldım" çipi GÖSTERİLMEZ — çip dili araç çağrılarına aittir ve
//  ayıklama sohbet turunun parçası değildir (spec §6.2).
//

import SwiftUI
import SwiftData

struct HafizaPanosu: View {
    @Query(sort: \HafizaNotu.olusturulma, order: .reverse)
    private var notlar: [HafizaNotu]

    /// Kaynak sohbet tarihini gösterebilmek için. Sohbet silinmişse kaynak
    /// satırı gizlenir, not kalır (spec §7).
    @Query private var sohbetler: [Sohbet]

    @Environment(\.modelContext) private var kayit
    @Environment(\.dismiss) private var kapat

    @State private var duzenlenen: HafizaNotu?
    @State private var silmeOnayi = false
    /// Yazma hatasının kullanıcıya görünen karşılığı.
    @State private var uyariMetni: String?

    var body: some View {
        NavigationStack {
            liste
                .background(Renk.zemin)
                .navigationTitle("Hafıza")
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItem(placement: .topBarLeading) {
                        Button("Kapat") { kapat() }
                            .font(Yazi.cip())
                            .foregroundStyle(Renk.gri)
                    }
                }
                .sheet(item: $duzenlenen) { not in
                    HafizaDuzenleyici(not: not, kaydet: guncelle)
                }
                .confirmationDialog("Hafızadaki her şey silinsin mi?",
                                    isPresented: $silmeOnayi,
                                    titleVisibility: .visible) {
                    Button("Sil", role: .destructive) { hepsiniSil() }
                    Button("Vazgeç", role: .cancel) { }
                } message: {
                    Text("Öğrenilen tüm notlar silinir. Sohbetler kalır. Bu işlem geri alınamaz.")
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
                if notlar.isEmpty {
                    bosDurum
                } else {
                    Text("Bu notlar cihazından çıkmaz. İlgili olduklarında sirr onlara bakar.")
                        .font(Yazi.cip())
                        .foregroundStyle(Renk.gri)
                        .padding(.bottom, Olcek.s1)
                        .hafizaSatiri()
                    if notlar.count >= HafizaNotu.toplamTavan {
                        Text("Hafıza dolu — yeni not eklenmiyor. Yer açmak için birkaçını sil.")
                            .font(Yazi.cip())
                            .foregroundStyle(Renk.hata)
                            .padding(.bottom, Olcek.s1)
                            .hafizaSatiri()
                    }
                }
            }

            ForEach(notlar) { not in
                Button { duzenlenen = not } label: { satir(not) }
                    .buttonStyle(.plain)
                    .hafizaSatiri()
            }
            .onDelete { indeksler in
                // Önce nesneleri topla: canlı @Query üzerinde indeksle silmek kayma üretir.
                for not in indeksler.map({ notlar[$0] }) { sil(not) }
            }

            if !notlar.isEmpty {
                Section {
                    hepsiniSilSatiri.hafizaSatiri()
                }
            }
        }
        .listStyle(.plain)
        .scrollContentBackground(.hidden)
    }

    private func satir(_ not: HafizaNotu) -> some View {
        VStack(alignment: .leading, spacing: Olcek.s1) {
            Text(not.metin)
                .font(Yazi.kullanici())
                .foregroundStyle(Renk.murekkep)
                .multilineTextAlignment(.leading)
                .lineLimit(3)
            HStack(spacing: Olcek.s2) {
                Text(Self.turEtiketi(not.tur))
                    .font(Yazi.cip())
                    .foregroundStyle(Renk.gri)
                if let kaynak = kaynakMetni(not) {
                    Text(kaynak)
                        .font(Yazi.cip())
                        .foregroundStyle(Renk.soluk)
                        .lineLimit(1)
                }
                if !not.aktif {
                    Text("kapalı")
                        .font(Yazi.cip())
                        .foregroundStyle(Renk.soluk)
                }
            }
        }
        .padding(.vertical, Olcek.s3)
        .padding(.horizontal, Olcek.s4)
        .frame(maxWidth: .infinity, alignment: .leading)
        .contentShape(Rectangle())
        .overlay(
            RoundedRectangle(cornerRadius: Olcek.s4, style: .continuous)
                .stroke(Renk.cizgi, lineWidth: Olcek.hairline)
        )
        // Üç ayrı metin değil, tek not satırı. "kapalı" rozeti duruma dönüşür.
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(Text(verbatim: "\(not.metin). \(Self.turEtiketi(not.tur))"))
        .accessibilityValue(not.aktif ? Text("Açık") : Text("Kapalı"))
        .accessibilityHint(Text("Düzenlemek için çift dokun."))
        .accessibilityAddTraits(.isButton)
    }

    private var hepsiniSilSatiri: some View {
        Button(role: .destructive) { silmeOnayi = true } label: {
            HStack(spacing: Olcek.s2) {
                Text("Hepsini sil")
                Spacer(minLength: 0)
            }
            .font(Yazi.kullanici())
            .foregroundStyle(Renk.hata)
            .padding(.vertical, Olcek.s3)
            .padding(.horizontal, Olcek.s4)
            .contentShape(Rectangle())
            .overlay(
                RoundedRectangle(cornerRadius: Olcek.s4, style: .continuous)
                    .stroke(Renk.cizgi, lineWidth: Olcek.hairline)
            )
        }
        .buttonStyle(.plain)
        .accessibilityLabel(Text("Hafızadaki her şeyi sil"))
        .accessibilityHint(Text("Onay sorulur. Tüm notlar silinir."))
    }

    private var bosDurum: some View {
        VStack(alignment: .leading, spacing: Olcek.s2) {
            Text("sirr henüz bir şey öğrenmedi.")
                .font(Yazi.ketum())
                .foregroundStyle(Renk.murekkep)
            Text("Sohbetlerde kendinden bahsettikçe burada görünür — ve yalnızca burada durur.")
                .font(Yazi.cip())
                .foregroundStyle(Renk.gri)
        }
        .padding(.vertical, Olcek.s4)
        .hafizaSatiri()
    }

    // MARK: - Kaynak

    /// "12 Tem sohbetinden" — kaynak sohbet silinmişse nil (satır gizlenir).
    private func kaynakMetni(_ not: HafizaNotu) -> String? {
        guard let id = not.kaynakSohbetID,
              let sohbet = sohbetler.first(where: { $0.id == id }) else { return nil }
        return String(localized: "\(Self.tarih.string(from: sohbet.olusturulma)) sohbetinden")
    }

    private static let tarih: DateFormatter = {
        let f = DateFormatter()
        f.locale = Locale.current
        f.setLocalizedDateFormatFromTemplate("d MMM")
        return f
    }()

    static func turEtiketi(_ tur: HafizaTuru) -> String {
        switch tur {
        case .kimlik: String(localized: "kimlik")
        case .tercih: String(localized: "tercih")
        case .iliski: String(localized: "ilişki")
        case .olgu:   String(localized: "olgu")
        }
    }

    // MARK: - Kayıt

    private func guncelle(_ taslak: HafizaTaslagi) {
        guard let not = duzenlenen, !not.isDeleted else { return }
        not.metin = taslak.metin
        not.anahtarlarHam = taslak.anahtarlar
        not.aktif = taslak.aktif
        kaydet(String(localized: "Not kaydedilemedi"))
    }

    private func sil(_ not: HafizaNotu) {
        guard !not.isDeleted else { return }
        kayit.delete(not)
        // Silme yazılamazsa geri alınır: liste gerçeği göstersin, kullanıcı
        // sildiğini sanıp notun hâlâ enjekte edildiğini kaçırmasın.
        kaydet(String(localized: "Not silinemedi"), geriAl: true)
    }

    private func hepsiniSil() {
        // Silmeden ÖNCE nesneleri topla; silinmiş kayda sonradan dokunmak ölümcüldür.
        let hepsi = notlar
        guard !hepsi.isEmpty else { return }
        for not in hepsi where !not.isDeleted { kayit.delete(not) }
        kaydet(String(localized: "Notlar silinemedi"), geriAl: true)
    }

    /// Diske yazar ve depoyu tazeler — model bir sonraki mesajda yeni hali okur.
    /// Yazma hatası yutulmaz: kullanıcı notun kaydedilmediğini görür.
    private func kaydet(_ neden: String, geriAl: Bool = false) {
        do {
            try kayit.save()
        } catch {
            if geriAl { kayit.rollback() }
            uyariMetni = "\(neden): \(error.localizedDescription)"
        }
        // Hata olsa da depoyu bağlamın güncel haliyle eşitle — model ile ekran ayrışmasın.
        HafizaDeposu.yenile((try? kayit.fetch(FetchDescriptor<HafizaNotu>())) ?? [])
    }
}

/// Düzenleyiciden panoya dönen düz değer — silinmiş modele dokunmadan kaydetmek için.
struct HafizaTaslagi {
    var metin: String
    var anahtarlar: String
    var aktif: Bool
}

// MARK: - Düzenleyici

private struct HafizaDuzenleyici: View {
    let not: HafizaNotu
    let kaydet: (HafizaTaslagi) -> Void

    @Environment(\.dismiss) private var kapat
    @State private var metin = ""
    @State private var anahtarlar = ""
    @State private var aktif = true
    @State private var yuklendi = false

    private var kalan: Int { HafizaNotu.metinSiniri - metin.count }
    private var gecerliMi: Bool {
        !metin.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !anahtarlar.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && kalan >= 0
    }

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextEditor(text: $metin)
                        .font(Yazi.kullanici())
                        .frame(minHeight: 90)
                        .scrollContentBackground(.hidden)
                        .accessibilityLabel(Text("Not metni"))
                } header: {
                    HStack {
                        Text("Not")
                        Spacer()
                        Text("\(max(kalan, 0))")
                            .foregroundStyle(kalan < 0 ? Renk.hata : Renk.soluk)
                            .monospacedDigit()
                            // Çıplak sayı olarak okunmasın.
                            .accessibilityLabel(Text("Kalan karakter"))
                            .accessibilityValue(Text(verbatim: "\(max(kalan, 0))"))
                    }
                } footer: {
                    Text("Tek cümle, en fazla \(HafizaNotu.metinSiniri) karakter. Kendi sözlerinle düzeltebilirsin.")
                }

                Section {
                    TextField("yemek, restoran, akşam", text: $anahtarlar, axis: .vertical)
                        .font(Yazi.kullanici())
                        .lineLimit(1...3)
                        .accessibilityLabel(Text("Anahtar kelimeler"))
                        .accessibilityHint(Text("Virgülle ayır."))
                } header: {
                    Text("Anahtarlar")
                } footer: {
                    Text("Virgülle ayır. Bu kelimelerden biri mesajda geçince not sirr'e verilir.")
                }

                Section {
                    Toggle("Açık", isOn: $aktif)
                        .font(Yazi.kullanici())
                } footer: {
                    Text("Kapalı not saklanır ama sirr'e hiç verilmez.")
                }
            }
            .scrollContentBackground(.hidden)
            .background(Renk.zemin)
            .navigationTitle("Not")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button("Vazgeç") { kapat() }
                        .font(Yazi.cip())
                        .foregroundStyle(Renk.gri)
                }
                ToolbarItem(placement: .topBarTrailing) {
                    Button("Kaydet") {
                        kaydet(HafizaTaslagi(
                            metin: metin.trimmingCharacters(in: .whitespacesAndNewlines),
                            anahtarlar: anahtarlar,
                            aktif: aktif
                        ))
                        kapat()
                    }
                    .font(Yazi.cip())
                    .foregroundStyle(gecerliMi ? Renk.murekkep : Renk.soluk)
                    .disabled(!gecerliMi)
                }
            }
            .onAppear {
                guard !yuklendi, !not.isDeleted else { return }
                yuklendi = true
                metin = not.metin
                anahtarlar = not.anahtarlarHam
                aktif = not.aktif
            }
            // Sınırı yazarken uygula — kullanıcı sessizce sınırı aşıp kaybetmesin.
            .onChange(of: metin) { _, yeni in
                if yeni.count > HafizaNotu.metinSiniri {
                    metin = String(yeni.prefix(HafizaNotu.metinSiniri))
                }
            }
        }
    }
}

private extension View {
    /// Panodaki her satırın ortak zemini: sistem ayraçları ve dolgusu yok.
    func hafizaSatiri() -> some View {
        listRowBackground(Renk.zemin)
            .listRowSeparator(.hidden)
            .listRowInsets(EdgeInsets(top: Olcek.s1, leading: Olcek.s5,
                                      bottom: Olcek.s1, trailing: Olcek.s5))
    }
}
