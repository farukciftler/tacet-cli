//
//  BeceriPanosu.swift
//  ketum
//
//  Beceri panosu — kullanıcının kendi yazdığı kılavuzlar (SKILL.md karşılığı).
//  Bir beceri, tetikleyici kelimeleri mesajda geçtiğinde o sohbete bir kez
//  iliştirilir. Gövde sert sınırlı: pencere 4096 token, her karakter bütçedir.
//

import SwiftUI
import SwiftData

struct BeceriPanosu: View {
    @Query(sort: \KullaniciBecerisi.olusturulma, order: .reverse)
    private var beceriler: [KullaniciBecerisi]

    @Environment(\.modelContext) private var kayit
    @Environment(\.dismiss) private var kapat

    @State private var duzenlenen: KullaniciBecerisi?
    @State private var yeniAcik = false

    var body: some View {
        NavigationStack {
            liste
                .background(Renk.zemin)
                .navigationTitle("Beceriler")
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItem(placement: .topBarLeading) {
                        Button("Kapat") { kapat() }
                            .font(Yazi.cip())
                            .foregroundStyle(Renk.gri)
                    }
                }
                .sheet(isPresented: $yeniAcik) {
                    BeceriDuzenleyici(beceri: nil, kaydet: ekle)
                }
                .sheet(item: $duzenlenen) { beceri in
                    BeceriDuzenleyici(beceri: beceri, kaydet: guncelle)
                }
        }
    }

    // MARK: - Liste

    private var liste: some View {
        List {
            Section {
                if beceriler.isEmpty {
                    bosDurum
                } else {
                    Text("Tetikleyici kelimen mesajda geçtiğinde sirr bu kılavuzu okur.")
                        .font(Yazi.cip())
                        .foregroundStyle(Renk.gri)
                        .padding(.bottom, Olcek.s1)
                        .beceriSatiri()
                }
            }

            ForEach(beceriler) { beceri in
                Button { duzenlenen = beceri } label: { satir(beceri) }
                    .buttonStyle(.plain)
                    .beceriSatiri()
            }
            .onDelete { indeksler in
                // Önce nesneleri topla: canlı @Query üzerinde indeksle silmek kayma üretir.
                for beceri in indeksler.map({ beceriler[$0] }) { sil(beceri) }
            }

            Section {
                yeniSatir.beceriSatiri()
            }

            Section {
                Text("sirr'in hazır becerileri")
                    .font(Yazi.etiket())
                    .foregroundStyle(Renk.soluk)
                    .textCase(.uppercase)
                    .beceriSatiri()
                ForEach(BeceriDeposu.paket, id: \.ad) { beceri in
                    VStack(alignment: .leading, spacing: Olcek.s1) {
                        Text(beceri.ad)
                            .font(Yazi.kullanici())
                            .foregroundStyle(Renk.gri)
                        Text(beceri.tetikler.prefix(5).joined(separator: " · "))
                            .font(Yazi.cip())
                            .foregroundStyle(Renk.soluk)
                            .lineLimit(1)
                    }
                    .padding(.vertical, Olcek.s1)
                    .beceriSatiri()
                }
            }
        }
        .listStyle(.plain)
        .scrollContentBackground(.hidden)
    }

    private func satir(_ beceri: KullaniciBecerisi) -> some View {
        VStack(alignment: .leading, spacing: Olcek.s1) {
            HStack(spacing: Olcek.s2) {
                Text(beceri.ad)
                    .font(Yazi.kullanici())
                    .foregroundStyle(Renk.murekkep)
                    .lineLimit(1)
                if !beceri.aktif {
                    Text("kapalı")
                        .font(Yazi.cip())
                        .foregroundStyle(Renk.soluk)
                }
            }
            Text(beceri.ozet)
                .font(Yazi.cip())
                .foregroundStyle(Renk.gri)
                .lineLimit(1)
            Text(beceri.govde)
                .font(Yazi.cip())
                .foregroundStyle(Renk.soluk)
                .lineLimit(2)
        }
        .padding(.vertical, Olcek.s3)
        .padding(.horizontal, Olcek.s4)
        .frame(maxWidth: .infinity, alignment: .leading)
        .contentShape(Rectangle())
        .overlay(
            RoundedRectangle(cornerRadius: Olcek.s4, style: .continuous)
                .stroke(Renk.cizgi, lineWidth: Olcek.hairline)
        )
    }

    private var yeniSatir: some View {
        Button { yeniAcik = true } label: {
            HStack(spacing: Olcek.s2) {
                Image(systemName: "plus")
                Text("Yeni beceri")
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
            Text("Henüz beceri yok.")
                .font(Yazi.ketum())
                .foregroundStyle(Renk.murekkep)
            Text("sirr'e kendi çalışma kuralını öğret: ne zaman devreye gireceğini ve ne yapacağını yaz.")
                .font(Yazi.cip())
                .foregroundStyle(Renk.gri)
        }
        .padding(.vertical, Olcek.s4)
        .beceriSatiri()
    }

    // MARK: - Kayıt

    private func ekle(_ taslak: BeceriTaslagi) {
        let yeni = KullaniciBecerisi(ad: taslak.ad, tetiklerHam: taslak.tetikler, govde: taslak.govde)
        yeni.aktif = taslak.aktif
        kayit.insert(yeni)
        kaydet()
    }

    private func guncelle(_ taslak: BeceriTaslagi) {
        guard let beceri = duzenlenen, !beceri.isDeleted else { return }
        beceri.ad = taslak.ad
        beceri.tetiklerHam = taslak.tetikler
        beceri.govde = taslak.govde
        beceri.aktif = taslak.aktif
        kaydet()
    }

    private func sil(_ beceri: KullaniciBecerisi) {
        guard !beceri.isDeleted else { return }
        kayit.delete(beceri)
        kaydet()
    }

    /// Diske yazar ve depoyu tazeler — model bir sonraki mesajda yeni hali okur.
    private func kaydet() {
        try? kayit.save()
        BeceriDeposu.kullaniciyiYenile(
            (try? kayit.fetch(FetchDescriptor<KullaniciBecerisi>())) ?? []
        )
    }
}

/// Düzenleyiciden panoya dönen düz değer — silinmiş modele dokunmadan kaydetmek için.
struct BeceriTaslagi {
    var ad: String
    var tetikler: String
    var govde: String
    var aktif: Bool
}

// MARK: - Düzenleyici

private struct BeceriDuzenleyici: View {
    let beceri: KullaniciBecerisi?
    let kaydet: (BeceriTaslagi) -> Void

    @Environment(\.dismiss) private var kapat
    @State private var ad = ""
    @State private var tetikler = ""
    @State private var govde = ""
    @State private var aktif = true

    private var kalan: Int { KullaniciBecerisi.govdeSiniri - govde.count }
    private var gecerliMi: Bool {
        !ad.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !tetikler.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !govde.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && kalan >= 0
    }

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextField("Fatura takibi", text: $ad)
                        .font(Yazi.kullanici())
                } header: {
                    Text("Ad")
                }

                Section {
                    TextField("fatura, gider, harcama", text: $tetikler, axis: .vertical)
                        .font(Yazi.kullanici())
                        .lineLimit(1...3)
                } header: {
                    Text("Tetikleyiciler")
                } footer: {
                    Text("Virgülle ayır. Bu kelimelerden biri mesajda geçince beceri devreye girer.")
                }

                Section {
                    TextEditor(text: $govde)
                        .font(Yazi.kullanici())
                        .frame(minHeight: 160)
                        .scrollContentBackground(.hidden)
                } header: {
                    HStack {
                        Text("Kılavuz")
                        Spacer()
                        Text("\(max(kalan, 0))")
                            .foregroundStyle(kalan < 0 ? Renk.hata : Renk.soluk)
                            .monospacedDigit()
                    }
                } footer: {
                    Text("Kısa ve emir kipinde yaz — en fazla \(KullaniciBecerisi.govdeSiniri) karakter. Örnek: “Fatura sorulduğunda önce arama aracıyla notlarda ara, sonra tutarı hesapla aracına ver.”")
                }

                Section {
                    Toggle("Açık", isOn: $aktif)
                        .font(Yazi.kullanici())
                }
            }
            .scrollContentBackground(.hidden)
            .background(Renk.zemin)
            .navigationTitle(beceri == nil ? "Yeni beceri" : "Beceri")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button("Vazgeç") { kapat() }
                        .font(Yazi.cip())
                        .foregroundStyle(Renk.gri)
                }
                ToolbarItem(placement: .topBarTrailing) {
                    Button("Kaydet") {
                        kaydet(BeceriTaslagi(
                            ad: ad.trimmingCharacters(in: .whitespacesAndNewlines),
                            tetikler: tetikler,
                            govde: govde.trimmingCharacters(in: .whitespacesAndNewlines),
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
                guard let beceri, ad.isEmpty else { return }
                ad = beceri.ad
                tetikler = beceri.tetiklerHam
                govde = beceri.govde
                aktif = beceri.aktif
            }
            // Sınırı yazarken uygula — kullanıcı sessizce sınırı aşıp kaybetmesin.
            .onChange(of: govde) { _, yeni in
                if yeni.count > KullaniciBecerisi.govdeSiniri {
                    govde = String(yeni.prefix(KullaniciBecerisi.govdeSiniri))
                }
            }
        }
    }
}

private extension View {
    /// Panodaki her satırın ortak zemini: sistem ayraçları ve dolgusu yok.
    func beceriSatiri() -> some View {
        listRowBackground(Renk.zemin)
            .listRowSeparator(.hidden)
            .listRowInsets(EdgeInsets(top: Olcek.s1, leading: Olcek.s5,
                                      bottom: Olcek.s1, trailing: Olcek.s5))
    }
}
