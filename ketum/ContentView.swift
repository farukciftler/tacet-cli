//
//  ContentView.swift
//  ketum
//
//  Kök görünüm. Aktif sohbeti, paylaşılan ModelServisi'ni ve geçmiş listesini yönetir.
//  Yeni sohbet açma ve eskilere erişim buradan (spec §4.7).
//

import SwiftUI
import SwiftData

struct ContentView: View {
    @Environment(\.modelContext) private var kayit
    @Environment(\.scenePhase) private var sahne
    @Query(sort: \Sohbet.guncelleme, order: .reverse) private var sohbetler: [Sohbet]

    @Query private var nobetler: [Nobet]
    @Query private var beceriler: [KullaniciBecerisi]

    @State private var servis = ModelServisi()
    @State private var aktifID: UUID?
    /// Tek sunum kanalı: aynı anda yalnız bir sayfa açık olabilir. Ayrı ayrı
    /// `isPresented` bayrakları kullanıldığında "kapat + aç" aynı runloop turuna
    /// düştüğü için ikinci sunum sessizce düşüyordu.
    @State private var sayfa: Sayfa?
    /// Nöbet tazeleme sürüyor mu — eş zamanlı ikinci akışı engeller.
    @State private var tazeleniyor = false

    /// Aktif sohbet — seçili yoksa en son güncellenen.
    private var aktif: Sohbet? {
        if let aktifID, let bulunan = sohbetler.first(where: { $0.id == aktifID }) {
            return bulunan
        }
        return sohbetler.first
    }

    var body: some View {
        Group {
            if let aktif {
                SohbetGorunumu(
                    sohbet: aktif,
                    servis: servis,
                    gecmisAc: { sayfa = .gecmis },
                    yeniSohbet: yeniSohbetBaslat
                )
                .id(aktif.id)   // sohbet değişince görünüm durumu (soru, akış) sıfırlanır
            } else {
                Color.clear.onAppear(perform: ilkSohbetiSagla)
            }
        }
        .task {
            // Kullanıcı becerilerini modele açılışta tanıt (pano kaydettikçe de tazeler).
            BeceriDeposu.kullaniciyiYenile(beceriler)
            await nobetleriTazele()
        }
        .onChange(of: sahne) { _, yeni in
            // Uygulama arka plandan öne gelince de tazele — .task yalnız ilk kez çalışır.
            guard yeni == .active else { return }
            Task { await nobetleriTazele() }
        }
        .sheet(item: $sayfa) { acik in
            switch acik {
            case .gecmis:
                SohbetListesi(
                    sohbetler: sohbetler,
                    aktifID: aktif?.id,
                    sec: { s in
                        aktifID = s.id
                        servis.sohbetiSifirla()
                        sayfa = nil
                    },
                    sil: silSohbet,
                    yeni: {
                        sayfa = nil
                        yeniSohbetBaslat()
                    },
                    // Tek atama: liste kapanır, hedef sayfa aynı geçişte sunulur.
                    nobetlerAc: { sayfa = .nobetler },
                    becerilerAc: { sayfa = .beceriler },
                    ayarlarAc: { sayfa = .ayarlar }
                )
            case .nobetler:
                NobetPanosu(servis: servis)
            case .beceriler:
                BeceriPanosu()
            case .ayarlar:
                Ayarlar(gecmisiTemizle: gecmisiTemizle)
            }
        }
    }

    /// Kökten sunulabilen sayfalar. `.sheet(item:)` ile tek kanaldan gider.
    private enum Sayfa: Identifiable {
        case gecmis, nobetler, beceriler, ayarlar
        var id: Self { self }
    }

    /// Nöbet bağlamına SwiftData'yı verir ve bugün çalışmamış nöbetleri hazırlar.
    /// Aynı anda tek akış çalışır: `gerekliyseCalistir` "bugün çalıştı mı" kontrolünden
    /// sonra beklediği için (izin diyaloğu dahil), koruma olmadan ikinci bir çağrı da
    /// guard'ı geçip aynı gün için ikinci kayıt yazabilirdi. Sürerken gelen çağrı atlanır.
    private func nobetleriTazele() async {
        guard !tazeleniyor else { return }
        tazeleniyor = true
        defer { tazeleniyor = false }
        servis.nobetBaglami.kayit = kayit
        await servis.nobetBaglami.servis.gerekliyseCalistir(nobetler, kayit: kayit)
    }

    // MARK: - Sohbet yönetimi

    private func ilkSohbetiSagla() {
        guard sohbetler.isEmpty else { return }
        bosSohbetAc()
    }

    /// Boş bir sohbet yazar ve aktif yapar. Tek yerden geçmesi önemli:
    /// geçmiş temizlendikten sonra da ekranda geçerli bir sohbet kalmalı.
    private func bosSohbetAc() {
        let yeni = Sohbet()
        kayit.insert(yeni)
        try? kayit.save()
        aktifID = yeni.id
    }

    private func yeniSohbetBaslat() {
        // Aktif sohbet zaten boşsa yeni açma — tek boş sohbet yeter.
        if let aktif, aktif.bosMu {
            aktifID = aktif.id
            servis.sohbetiSifirla()
            return
        }
        let yeni = Sohbet()
        kayit.insert(yeni)
        try? kayit.save()
        aktifID = yeni.id
        servis.sohbetiSifirla()
    }

    private func silSohbet(_ sohbet: Sohbet) {
        // Kimliği silmeden önce al — silinen nesneye erişmek ölümcül hata.
        // Sıradaki aktif de silmeden ÖNCE hesaplanır: @Query dizisi silme anında
        // henüz tazelenmemiş olabilir, sonradan gezmek silinmiş örneğe dokunur.
        let silinenID = sohbet.id
        let siliniyorAktif = (silinenID == aktif?.id)
        let sonrakiID = sohbetler.first(where: { $0.id != silinenID })?.id
        kayit.delete(sohbet)
        try? kayit.save()
        if siliniyorAktif {
            aktifID = sonrakiID
            servis.sohbetiSifirla()
        }
    }

    /// Tüm sohbet geçmişini siler. Nöbetler, nöbet kayıtları ve üretilmiş belgeler
    /// kalır — onları silmek kullanıcının ayrı kararı.
    private func gecmisiTemizle() {
        // Silinecek nesneler silmeden ÖNCE tutulur: canlı @Query dizisi silme anında
        // tazelenmemiş olabilir, sonradan gezmek silinmiş örneğe dokunur (ölümcül).
        // Mesajlar Sohbet.mesajlar üzerindeki cascade kuralıyla birlikte gider.
        let silinecekler = sohbetler
        aktifID = nil
        servis.sohbetiSifirla()
        for sohbet in silinecekler { kayit.delete(sohbet) }
        try? kayit.save()
        // Ekranda geçerli bir durum kalsın diye hemen boş bir sohbet açılır.
        bosSohbetAc()
    }
}

#Preview {
    ContentView()
        .modelContainer(for: [Sohbet.self, Mesaj.self, Nobet.self, NobetKaydi.self,
                              KullaniciBecerisi.self], inMemory: true)
}
