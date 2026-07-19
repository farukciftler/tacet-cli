//
//  IzinBolumu.swift
//  ketum
//
//  Ayarlar içindeki izin bölümü. Yalnızca gösterir: izni araçlar, kullanıcı
//  gerçekten bir şey istediği anda ister — ayar ekranının izin penceresi
//  açması, kullanıcının istemediği bir şeyi istemek olurdu.
//

import SwiftUI
import EventKit
import Contacts
import UserNotifications

struct IzinBolumu: View {
    @Environment(\.scenePhase) private var evre

    @State private var takvim: IzinDurumu = .bilinmiyor
    @State private var hatirlatici: IzinDurumu = .bilinmiyor
    @State private var kisiler: IzinDurumu = .bilinmiyor
    @State private var bildirim: IzinDurumu = .bilinmiyor

    init() {}

    /// Kartın altında duran dürüst açıklama; başlık ve çerçeve Ayarlar'ın işi.
    static let aciklama: LocalizedStringKey =
        "İzinler yalnızca sen bir şey istediğin anda kullanılır ve veri cihazdan çıkmaz; değiştirmek için iOS Ayarlar'a gidilir."

    var body: some View {
        VStack(spacing: 0) {
            satir(ad: String(localized: "Takvim"), durum: takvim)
            ayrac
            satir(ad: String(localized: "Hatırlatıcılar"), durum: hatirlatici)
            ayrac
            satir(ad: String(localized: "Kişiler"), durum: kisiler)
            ayrac
            satir(ad: String(localized: "Bildirimler"), durum: bildirim)
            ayrac
            ayarlarSatiri
        }
        .task { await tazele() }
        .onChange(of: evre) { _, yeni in
            // Kullanıcı iOS Ayarlar'dan dönünce ekran bayat kalmasın.
            if yeni == .active { Task { await tazele() } }
        }
    }

    // MARK: - Satırlar

    private var ayrac: some View {
        Rectangle()
            .fill(Renk.cizgi)
            .frame(height: Olcek.hairline)
    }

    private func satir(ad: String, durum: IzinDurumu) -> some View {
        HStack(spacing: Olcek.s2) {
            Text(ad)
                .font(Yazi.kullanici())
                .foregroundStyle(Renk.murekkep)
            Spacer(minLength: Olcek.s3)
            // İm yalnızca göz için; durumu zaten yanındaki kelime söylüyor.
            Image(systemName: durum.im)
                .font(Yazi.cip())
                .foregroundStyle(Renk.soluk)
                .accessibilityHidden(true)
            Text(durum.ad)
                .font(Yazi.cip())
                .foregroundStyle(Renk.gri)
        }
        .padding(.vertical, Olcek.s3)
        .padding(.horizontal, Olcek.s4)
        // "Takvim, Verilmedi" tek parça okunur; üç ayrı öğe olarak gezilmez.
        .accessibilityElement(children: .combine)
    }

    private var ayarlarSatiri: some View {
        Button {
            if let url = URL(string: UIApplication.openSettingsURLString) {
                UIApplication.shared.open(url)
            }
        } label: {
            HStack(spacing: Olcek.s2) {
                Text("iOS Ayarlar'ı aç")
                    .font(Yazi.kullanici())
                    .foregroundStyle(Renk.murekkep)
                Spacer(minLength: Olcek.s3)
                Image(systemName: "arrow.up.forward")
                    .font(Yazi.cip())
                    .foregroundStyle(Renk.soluk)
                    .accessibilityHidden(true)
            }
            .padding(.vertical, Olcek.s3)
            .padding(.horizontal, Olcek.s4)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityElement(children: .combine)
        .accessibilityHint(Text("Uygulamadan çıkıp iOS Ayarlar'ı açar"))
    }

    // MARK: - Okuma

    private func tazele() async {
        takvim = IzinDurumu(EKEventStore.authorizationStatus(for: .event))
        hatirlatici = IzinDurumu(EKEventStore.authorizationStatus(for: .reminder))
        kisiler = IzinDurumu(CNContactStore.authorizationStatus(for: .contacts))
        let ayarlar = await UNUserNotificationCenter.current().notificationSettings()
        bildirim = IzinDurumu(ayarlar.authorizationStatus)
    }
}

// MARK: - Durum

/// Dört farklı sistem enum'unu tek, kelimeyle anlatılan duruma indirger.
private enum IzinDurumu {
    case verildi, verilmedi, sorulmadi, kisitli, bilinmiyor

    var ad: String {
        switch self {
        case .verildi:    String(localized: "Verildi")
        case .verilmedi:  String(localized: "Verilmedi")
        case .sorulmadi:  String(localized: "Henüz sorulmadı")
        case .kisitli:    String(localized: "Kısıtlı")
        case .bilinmiyor: String(localized: "Okunuyor…")
        }
    }

    /// Renksiz im — durum kelimeyle anlatılır, im yalnızca göz için.
    var im: String {
        switch self {
        case .verildi:    "checkmark"
        case .verilmedi:  "xmark"
        case .sorulmadi:  "minus"
        case .kisitli:    "lock"
        case .bilinmiyor: "ellipsis"
        }
    }

    init(_ durum: EKAuthorizationStatus) {
        switch durum {
        case .fullAccess:    self = .verildi
        case .writeOnly:     self = .kisitli
        case .notDetermined: self = .sorulmadi
        case .restricted:    self = .kisitli
        case .denied:        self = .verilmedi
        @unknown default:    self = .bilinmiyor
        }
    }

    init(_ durum: CNAuthorizationStatus) {
        switch durum {
        case .authorized:    self = .verildi
        case .limited:       self = .kisitli
        case .notDetermined: self = .sorulmadi
        case .restricted:    self = .kisitli
        case .denied:        self = .verilmedi
        @unknown default:    self = .bilinmiyor
        }
    }

    init(_ durum: UNAuthorizationStatus) {
        switch durum {
        case .authorized, .ephemeral: self = .verildi
        case .provisional:            self = .kisitli
        case .notDetermined:          self = .sorulmadi
        case .denied:                 self = .verilmedi
        @unknown default:             self = .bilinmiyor
        }
    }
}
