//
//  OnaySayfasi.swift
//  ketum
//
//  Paylaşım onayı (mcp §3.3, §4). Kirli oturumda cihaz dışına veri çıkmadan
//  önce kullanıcı GÖNDERİLECEK İÇERİĞİN AYNISINI görür ve karar verir.
//  Ton: dramatize etmez, korkutmaz; ne gideceğini gösterir ve sorar.
//

import SwiftUI

struct OnaySayfasi: View {
    /// Bağlantı/sunucu adı — "ev sunucusu".
    let kaynak: String
    let aracAdi: String
    /// Gönderilecek argümanların aynısı; kategori özeti değil.
    let icerik: String
    /// true = gönder, false = gönderme. Kapatmak da false üretir.
    let karar: (Bool) -> Void

    @Environment(\.dismiss) private var kapat

    /// Kapanış yolu ne olursa olsun karar tam bir kez bildirilir: swipe ile
    /// kapatma da "Gönderme"dir, ama düğmeye basıldıysa ikinci kez bildirilmez.
    @State private var kararVerildi = false

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: Olcek.s4) {
                    Text("\(kaynak) · \(aracAdi)")
                        .font(Yazi.kullanici())
                        .foregroundStyle(Renk.murekkep)
                        .fixedSize(horizontal: false, vertical: true)

                    VStack(alignment: .leading, spacing: Olcek.s2) {
                        Text("SUNUCUNA GİDİYOR:")
                            .font(Yazi.etiket())
                            .tracking(0.6)
                            .foregroundStyle(Renk.soluk)

                        Text(icerik)
                            .font(.system(.footnote, design: .monospaced))
                            .foregroundStyle(Renk.murekkep)
                            .textSelection(.enabled)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .padding(Olcek.s3)
                            .background(Renk.dolgu)
                            .clipShape(RoundedRectangle(cornerRadius: Olcek.cipKose,
                                                        style: .continuous))
                    }
                    .accessibilityElement(children: .combine)
                    .accessibilityLabel(Text("Sunucuya gidecek içerik"))
                    .accessibilityValue(Text(icerik))

                    Text("Göndermezsen sirr bu adımı atlar ve yapamadığını söyler.")
                        .font(Yazi.cip())
                        .foregroundStyle(Renk.gri)
                        .fixedSize(horizontal: false, vertical: true)

                    dugmeler
                }
                .padding(.horizontal, Olcek.s5)
                .padding(.vertical, Olcek.s4)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .background(Renk.zemin)
            .navigationTitle(Text("Gönderilsin mi"))
            .navigationBarTitleDisplayMode(.inline)
        }
        .presentationDetents([.medium, .large])
        .interactiveDismissDisabled(false)
        .onDisappear {
            // Kapatmak = "Gönderme".
            bildir(false)
        }
    }

    private var dugmeler: some View {
        HStack(spacing: Olcek.s3) {
            dugme(baslik: Text("Gönderme"), vurgulu: false) { bildir(false) }
            dugme(baslik: Text("Gönder"), vurgulu: true) { bildir(true) }
        }
        .padding(.top, Olcek.s2)
    }

    // Vurgu rengi yok: birincil eylem dolguyla, ikincil hairline çerçeveyle ayrışır.
    private func dugme(baslik: Text, vurgulu: Bool, eylem: @escaping () -> Void) -> some View {
        Button(action: eylem) {
            baslik
                .font(Yazi.kullanici())
                .foregroundStyle(vurgulu ? Renk.zemin : Renk.murekkep)
                .frame(maxWidth: .infinity)
                .frame(minHeight: Olcek.dokunmaHedefi)
                .background {
                    if vurgulu {
                        RoundedRectangle(cornerRadius: Olcek.cipKose, style: .continuous)
                            .fill(Renk.murekkep)
                    } else {
                        RoundedRectangle(cornerRadius: Olcek.cipKose, style: .continuous)
                            .stroke(Renk.cizgi, lineWidth: Olcek.hairline)
                    }
                }
                .contentShape(RoundedRectangle(cornerRadius: Olcek.cipKose, style: .continuous))
        }
        .buttonStyle(.plain)
    }

    private func bildir(_ kabul: Bool) {
        guard !kararVerildi else { return }
        kararVerildi = true
        karar(kabul)
        kapat()
    }
}

#Preview {
    Color.clear.sheet(isPresented: .constant(true)) {
        OnaySayfasi(
            kaynak: "ev sunucusu",
            aracAdi: "issue_ac",
            icerik: "{\n  \"baslik\": \"Diş hekimi randevusu\",\n  \"saat\": \"10:00\"\n}",
            karar: { _ in }
        )
    }
}
