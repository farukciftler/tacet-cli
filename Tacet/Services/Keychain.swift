//
//  Keychain.swift
//  Tacet
//
//  The single storage place for bearer keys (mcp-connection-spec §5.8).
//
//  `kSecAttrAccessibleWhenUnlockedThisDeviceOnly`: the record DOES NOT enter the iCloud
//  Keychain or the device backup; it is readable only on this device and only while the
//  device is unlocked. The SwiftData store is not encrypted and can end up in a backup,
//  which is why `Connection` holds only the reference that finds the record here.
//
//  The token is NEVER SHOWN in the UI again: `read` exists only for `MCPClient`, which puts
//  the Authorization header on the network request; the view layer never calls it.
//

import Foundation
import Security

enum Keychain {

    /// The Keychain service field — so it does not collide with the app's other records.
    ///
    /// THE VALUE STAYS AS IT IS, in Turkish, AND THAT IS DELIBERATE: this is not app text but
    /// a PERSISTED LOOKUP KEY on the user's device. Every token already saved is filed under
    /// this exact string. Rename it and `read` finds nothing for any existing connection: the
    /// connection silently becomes unauthorized, and because the secret is never shown again
    /// the user cannot even retype it. Moving it is a data migration (read under the old
    /// service, rewrite under the new one, then delete), not a translation.
    private static let service = "com.tacet.baglanti"

    /// Produces a unique reference for a new connection. This value lives in `Connection`
    /// (that is, in SwiftData); it is not the secret itself.
    ///
    /// The `baglanti-` prefix is FROZEN for the same reason as `service`: it is part of the
    /// reference already stored in existing records.
    static func newRef() -> String { "baglanti-" + UUID().uuidString }

    /// Writes the key (overwriting an existing one). If empty text is given the record is
    /// deleted — so that "clear the key" needs no separate call.
    @discardableResult
    static func write(_ key: String, ref: String) -> Bool {
        let clean = key.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !clean.isEmpty else { return delete(ref: ref) }
        guard let data = clean.data(using: .utf8) else { return false }

        let identity: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: ref,
        ]

        // ADD FIRST, UPDATE ON CONFLICT — NOT delete-then-add.
        //
        // The record used to be deleted first. If `SecItemAdd` then failed (a background write
        // on a locked device returns `errSecInteractionNotAllowed`; over quota it returns
        // `errSecDiskFull`) the user's WORKING key was gone too and all that was left was
        // `false`: the connection silently became unauthorized, and because the secret can
        // never be seen again the user could not rewrite it. A FAILED WRITE MUST NOT DAMAGE
        // THE RECORD ALREADY IN PLACE.
        var addition = identity
        addition[kSecValueData as String] = data
        addition[kSecAttrAccessible as String] = kSecAttrAccessibleWhenUnlockedThisDeviceOnly
        let addStatus = SecItemAdd(addition as CFDictionary, nil)
        if addStatus == errSecSuccess { return true }
        guard addStatus == errSecDuplicateItem else { return false }

        // The record already exists: the value AND the accessibility class are updated
        // together. If the class were left out of the update an old record could stay in the
        // wrong class — that was the real reason for the delete-then-write pattern; putting
        // `kSecAttrAccessible` into the `SecItemUpdate` attribute dictionary gives the same
        // guarantee.
        let update: [String: Any] = [
            kSecValueData as String: data,
            kSecAttrAccessible as String: kSecAttrAccessibleWhenUnlockedThisDeviceOnly,
        ]
        return SecItemUpdate(identity as CFDictionary, update as CFDictionary) == errSecSuccess
    }

    /// Reads the key. nil if there is no record or the device is locked.
    /// ONLY the network layer calls this; it never goes back to the UI.
    static func read(ref: String) -> String? {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: ref,
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne,
        ]
        var result: CFTypeRef?
        guard SecItemCopyMatching(query as CFDictionary, &result) == errSecSuccess,
              let data = result as? Data else { return nil }
        return String(data: data, encoding: .utf8)
    }

    /// Says whether the record exists without reading the secret — `ConnectionDetail`
    /// shows its "key saved" state with this and never sees the value.
    ///
    /// A LOCKED DEVICE ALSO ANSWERS `false`: the record is filed under
    /// `WhenUnlockedThisDeviceOnly` and is not even visible to a search while the device
    /// is locked. That is harmless for the one caller (a foreground screen, so the device
    /// is unlocked), but it makes this UNUSABLE AS A DELETE CONDITION on a background
    /// path — "no record, drop the reference" would throw the user's working key away.
    static func exists(ref: String) -> Bool {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: ref,
            kSecMatchLimit as String: kSecMatchLimitOne,
        ]
        return SecItemCopyMatching(query as CFDictionary, nil) == errSecSuccess
    }

    /// Called when a connection is deleted. If the record is already gone that counts as
    /// success too.
    @discardableResult
    static func delete(ref: String) -> Bool {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: ref,
        ]
        let status = SecItemDelete(query as CFDictionary)
        return status == errSecSuccess || status == errSecItemNotFound
    }

    /// Every reference in this service field. It DOES NOT READ the secrets, it only fetches
    /// the names (no `kSecReturnData`) — it opens no leak surface.
    static func refs() -> [String] {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecReturnAttributes as String: true,
            kSecMatchLimit as String: kSecMatchLimitAll,
        ]
        var result: CFTypeRef?
        guard SecItemCopyMatching(query as CFDictionary, &result) == errSecSuccess,
              let records = result as? [[String: Any]] else { return [] }
        return records.compactMap { $0[kSecAttrAccount as String] as? String }
    }

    /// Drops records that no longer have an owner.
    ///
    /// A Keychain record lives INDEPENDENTLY of the SwiftData store: if the app is deleted and
    /// reinstalled, if the store is moved, or if a record was half-written in the past, the
    /// token stays there — and because no `Connection` points at it, it is neither read nor
    /// deleted. `TacetApp.purgeOrphanKeys` calls this at launch with the references of the
    /// live connections; every record not in that list is deleted.
    ///
    /// `valid` MUST BE THE REAL LIST. This function cannot tell "the user has no
    /// connections" from "the connection list could not be read": both arrive as an empty
    /// set, and the second one wipes every token the user has — irrecoverably, since a
    /// secret is never shown again. Deciding that is the caller's job, and the launch
    /// caller only runs when the store opened healthy.
    ///
    /// On a LOCKED device `refs()` returns nothing, so this deletes nothing rather than
    /// mistaking an invisible record for an orphan. A background launch simply skips the
    /// cleanup; the next foreground launch does it.
    /// - Returns: the number of records deleted.
    @discardableResult
    static func purgeOrphans(valid: Set<String>) -> Int {
        var deleted = 0
        for ref in refs() where !valid.contains(ref) {
            if delete(ref: ref) { deleted += 1 }
        }
        return deleted
    }
}
