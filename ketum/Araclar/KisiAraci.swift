import Foundation
import Contacts
import FoundationModels

// Kişilerde isimle arama yapıp telefon ve e-posta döndürür. Yalnızca okuma.
struct KisiAraci: KetumAraci {
    let name = "kisi"
    let description = "Looks up a contact by name and returns their phone number and email. Call this whenever the user asks for someone's number or email, in any language."

    weak var raporlayici: (any AracRaporlayici)?

    @Generable struct Arguments {
        @Guide(description: "Aranacak kişinin adı ya da soyadı")
        var isim: String
    }

    func call(arguments: Arguments) async -> String {
        await cipliCalis(ikon: "person", calisiyorMetni: Yerel.kisiAraniyor, hamGirdi: arguments.isim) {
            let store = CNContactStore()

            // İzin durumu: reddedilmişse çipi izinGerekli yap, throw etme.
            let durum = CNContactStore.authorizationStatus(for: .contacts)
            if durum == .denied || durum == .restricted {
                return AracSonucu(
                    cipMetni: Yerel.kisiIzni,
                    durum: .izinGerekli,
                    modeleDonen: "permission_required (user can grant access in Settings)"
                )
            }

            // Henüz sorulmadıysa izin iste.
            if durum == .notDetermined {
                let verildi = try await store.requestAccess(for: .contacts)
                if !verildi {
                    return AracSonucu(
                        cipMetni: Yerel.kisiIzni,
                        durum: .izinGerekli,
                        modeleDonen: "permission_required (user can grant access in Settings)"
                    )
                }
            }

            // Eşleşen kişileri getir.
            let anahtarlar: [CNKeyDescriptor] = [
                CNContactGivenNameKey as CNKeyDescriptor,
                CNContactFamilyNameKey as CNKeyDescriptor,
                CNContactPhoneNumbersKey as CNKeyDescriptor,
                CNContactEmailAddressesKey as CNKeyDescriptor
            ]
            let yordam = CNContact.predicateForContacts(matchingName: arguments.isim)
            let kisiler = try store.unifiedContacts(matching: yordam, keysToFetch: anahtarlar)

            if kisiler.isEmpty {
                return AracSonucu(
                    cipMetni: Yerel.kisiArandiYok,
                    durum: .okundu,
                    modeleDonen: "no_contact_found"
                )
            }

            // Her kişiyi tek satıra indirger: "Ad Soyad · numara/eposta".
            func satir(_ k: CNContact) -> String {
                let adSoyad = [k.givenName, k.familyName]
                    .filter { !$0.isEmpty }
                    .joined(separator: " ")
                var iletisim: [String] = []
                if let tel = k.phoneNumbers.first?.value.stringValue, !tel.isEmpty {
                    iletisim.append(tel)
                }
                if let posta = k.emailAddresses.first?.value as String?, !posta.isEmpty {
                    iletisim.append(posta)
                }
                let iletisimMetni = iletisim.joined(separator: " ")
                let ad = adSoyad.isEmpty ? "(isimsiz)" : adSoyad
                return iletisimMetni.isEmpty ? ad : "\(ad) · \(iletisimMetni)"
            }

            let ozet = kisiler.prefix(5).map(satir).joined(separator: "; ")
            let tam = kisiler.map(satir).joined(separator: "\n")

            return AracSonucu(
                cipMetni: Yerel.kisiArandi,
                durum: .okundu,
                modeleDonen: ozet,
                hamCikti: tam
            )
        }
    }
}
