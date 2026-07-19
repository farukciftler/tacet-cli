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
    var ayarlarAc: () -> Void = {}

    @Environment(\.dismiss) private var kapat

    var body: some View {
        NavigationStack {
            List {
                Button(action: nobetlerAc) {
                    HStack(spacing: Olcek.s3) {
                        Image(systemName: "moon.stars")
                            .font(.system(size: 15))
                            .foregroundStyle(Renk.gri)
                            .frame(width: 6)
                        Text("Nöbetler")
                            .font(Yazi.kullanici())
                            .foregroundStyle(Renk.murekkep)
                        Spacer()
                        Image(systemName: "chevron.right")
                            .font(.system(size: 12))
                            .foregroundStyle(Renk.soluk)
                    }
                    .padding(.vertical, Olcek.s1)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .listRowBackground(Renk.zemin)

                Button(action: becerilerAc) {
                    HStack(spacing: Olcek.s3) {
                        Image(systemName: "wand.and.stars")
                            .font(.system(size: 15))
                            .foregroundStyle(Renk.gri)
                            .frame(width: 6)
                        Text("Beceriler")
                            .font(Yazi.kullanici())
                            .foregroundStyle(Renk.murekkep)
                        Spacer()
                        Image(systemName: "chevron.right")
                            .font(.system(size: 12))
                            .foregroundStyle(Renk.soluk)
                    }
                    .padding(.vertical, Olcek.s1)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .listRowBackground(Renk.zemin)

                Button(action: ayarlarAc) {
                    HStack(spacing: Olcek.s3) {
                        Image(systemName: "gearshape")
                            .font(.system(size: 15))
                            .foregroundStyle(Renk.gri)
                            .frame(width: 6)
                        Text("Ayarlar")
                            .font(Yazi.kullanici())
                            .foregroundStyle(Renk.murekkep)
                        Spacer()
                        Image(systemName: "chevron.right")
                            .font(.system(size: 12))
                            .foregroundStyle(Renk.soluk)
                    }
                    .padding(.vertical, Olcek.s1)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .listRowBackground(Renk.zemin)

                ForEach(sohbetler) { sohbet in
                    Button {
                        sec(sohbet)
                    } label: {
                        satir(sohbet)
                    }
                    .buttonStyle(.plain)
                    .listRowBackground(Renk.zemin)
                }
                .onDelete { indeksler in
                    for i in indeksler { sil(sohbetler[i]) }
                }
            }
            .listStyle(.plain)
            .background(Renk.zemin)
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

    private func satir(_ sohbet: Sohbet) -> some View {
        HStack(spacing: Olcek.s3) {
            // Aktif sohbet için küçük mürekkep nokta (marka: renk değil, ağırlık/işaret).
            Circle()
                .fill(sohbet.id == aktifID ? Renk.murekkep : Color.clear)
                .frame(width: 6, height: 6)

            VStack(alignment: .leading, spacing: Olcek.s1) {
                Text(sohbet.baslik.isEmpty ? "Yeni sohbet" : sohbet.baslik)
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
