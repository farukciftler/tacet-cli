//
//  CihazVerisiSecici.swift
//  ketum
//
//  Bağlantı ekranlarının ortak "cihaz verisi" seçicisi — mcp-baglanti-spec §3.1.
//
//  Üç mod var. İlk ikisi kısıtlayıcı yöndedir ve sessizce uygulanır.
//  Üçüncüsü (`herZaman`) onay kapısını kapatır: SEÇİLDİĞİ ANDA uyarı modalı
//  çıkar ve kullanıcı vazgeçerse seçim eski değerinde kalır — modal kapanınca
//  ayarın sessizce değişmiş olması, tam da kapının anlamını yitirmesi olurdu.
//
//  Seçiliyken ekranda kalıcı bir durum satırı durur: kullanıcı ekrana günler
//  sonra girdiğinde bu bağlantının sormadığını modal olmadan da görmeli.
//

import SwiftUI

struct CihazVerisiSecici: View {
    @Binding var secim: CihazVerisiAyari
    var kapali: Bool = false

    @Environment(\.accessibilityReduceMotion) private var hareketiAzalt

    /// Kapıyı kapatma onayı bekleniyor mu.
    @State private var onayIsteniyor = false
    /// Reddedilen seçimden sonra segmenti kaynağa geri çizdirmek için.
    /// Binding'in `set`'i değeri değiştirmediğinde SwiftUI görünümü kendi
    /// başına geçersiz kılmayabiliyor; kimlik değişince kesin olarak çiziyor.
    @State private var tazeleme = 0

    var body: some View {
        VStack(alignment: .leading, spacing: Olcek.s2) {
            Picker("Cihaz verisi", selection: Binding(
                get: { secim },
                set: { istendi($0) }
            )) {
                Text("hiçbir zaman")
                    .tag(CihazVerisiAyari.hicbirZaman)
                    .accessibilityLabel(Text("Hiçbir zaman gönderme"))
                Text("her seferinde sor")
                    .tag(CihazVerisiAyari.herSeferindeSor)
                    .accessibilityLabel(Text("Her seferinde sor"))
                Text("her zaman")
                    .tag(CihazVerisiAyari.herZaman)
                    .accessibilityLabel(Text("Her zaman izin ver"))
            }
            .pickerStyle(.segmented)
            .id(tazeleme)
            .disabled(kapali)
            .accessibilityLabel(Text("Cihaz verisi paylaşımı"))
            .accessibilityHint(Text("Takvim, kişi ve belge gibi verilerin bu sunucuya gönderilip gönderilmeyeceğini belirler. Her zaman izin ver seçilirse onay sorulmaz."))

            Text(aciklama)
                .font(Yazi.cip())
                .foregroundStyle(Renk.gri)
                .fixedSize(horizontal: false, vertical: true)

            if secim.kapiyiAtlarMi { durumSatiri }
        }
        .alert(Text("Onay sormadan göndermeyi aç"), isPresented: $onayIsteniyor) {
            Button(role: .cancel) { onayIsteniyor = false } label: {
                Text("Vazgeç")
            }
            .accessibilityHint(Text("Ayar olduğu gibi kalır."))

            Button { onayla() } label: {
                Text("Her zaman izin ver")
            }
            .accessibilityHint(Text("Bu bağlantıya bundan sonra sorulmadan gönderilir."))
        } message: {
            Text(modalMetni)
        }
    }

    // MARK: - Parçalar

    /// Seçiliyken ekranda duran sakin durum satırı. Nokta ya da rozet yok:
    /// durum sözle söylenir.
    private var durumSatiri: some View {
        VStack(alignment: .leading, spacing: Olcek.s1) {
            Text("Bu bağlantıya onay sorulmadan gönderiliyor.")
                .font(Yazi.cip())
                .foregroundStyle(Renk.hata)
            Text("Giden her şeyi sohbetteki çipe dokunarak sonradan görebilirsin. Bu ayarı istediğin zaman geri alabilirsin.")
                .font(Yazi.cip())
                .foregroundStyle(Renk.gri)
        }
        .fixedSize(horizontal: false, vertical: true)
        .padding(.vertical, Olcek.s3)
        .padding(.horizontal, Olcek.s4)
        .frame(maxWidth: .infinity, alignment: .leading)
        .overlay(
            RoundedRectangle(cornerRadius: Olcek.s4, style: .continuous)
                .stroke(Renk.cizgi, lineWidth: Olcek.hairline)
        )
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isSummaryElement)
    }

    private var aciklama: LocalizedStringKey {
        switch secim {
        case .hicbirZaman:
            return "Cihazından okunan veri bu sunucuya hiç gönderilmez."
        case .herSeferindeSor:
            return "Cihazından okunan veri gönderilmeden önce her seferinde sana gösterilir."
        case .herZaman:
            return "Cihazından okunan veri, sana sorulmadan bu sunucuya gönderilebilir."
        }
    }

    /// Ne kaybedildiğini somut söyler; korkutmadan ama yumuşatmadan.
    private var modalMetni: LocalizedStringKey {
        """
        sirr bu bağlantıya veri gönderirken artık sana sormayacak.

        Aynı sohbette takvim, kişiler, notlar ya da belgeler kullanıldıysa, o araçlardan gelen içerik senin görmediğin bir argümanın içine girip sunucuya gidebilir.

        Sunucu ele geçirilirse ya da kötü niyetliyse, döndürdüğü metin modeli daha fazla veri göndermeye yönlendirebilir. Onay kapısı tam da bunu durduruyordu.

        Giden her şeyi sohbetteki çipe dokunarak sonradan görebilirsin: kalkan yalnızca ön onay, şeffaflık değil.

        Bu ayarı istediğin zaman geri alabilirsin.
        """
    }

    // MARK: - Eylemler

    /// Kısıtlayıcı yöne geçiş uyarı istemez; kapıyı kapatan geçiş ister.
    private func istendi(_ yeni: CihazVerisiAyari) {
        guard yeni != secim else { return }
        guard yeni.kapiyiAtlarMi else {
            secim = yeni
            return
        }
        // Değeri ŞİMDİ yazmıyoruz: onaylanmadan ayar değişmiş olmamalı.
        onayIsteniyor = true
        var islem = Transaction()
        islem.disablesAnimations = hareketiAzalt
        withTransaction(islem) { tazeleme &+= 1 }
    }

    private func onayla() {
        onayIsteniyor = false
        var islem = Transaction()
        islem.disablesAnimations = hareketiAzalt
        withTransaction(islem) {
            secim = .herZaman
            tazeleme &+= 1
        }
    }
}
