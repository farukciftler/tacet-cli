//
//  WebSearchSetting.swift
//  Tacet
//
//  The web search setting (web-search-spec §5.1): the user's own SearXNG root
//  address + on/off. There is no token, so no Keychain is needed; UserDefaults is
//  enough. The view layer can bind the same keys with @AppStorage — the key names
//  are defined here, in one place.
//
//  OFF BY DEFAULT (spec §2.1): the app ships no predefined server address. Only a
//  DEBUG build can read the developer example, and only if it was supplied through
//  a build flag. In the App Store build the field is empty.
//
//  Embedding a predefined server address was tried once and REVERTED: in a
//  single-user setup it looks harmless, but once the app ships to many users the
//  owner of that address turns into a data processor carrying everyone's search
//  queries, and the promise "your query never goes beyond your own server" loses
//  its meaning after the first user. The user enters the server.
//

import Foundation

enum WebSearchSetting {

    // MARK: - UserDefaults keys
    // View layer: @AppStorage(WebSearchSetting.activeKey)
    //
    // RENAMED WITH THE ENGLISH MIGRATION: a value stored under the old Turkish key
    // is not read. A user who had configured a server has to enter it again once —
    // the same trade-off as LanguagePreference, and the opposite of the Keychain
    // service string, which stays frozen because its secret is unrecoverable.

    static let activeKey = "tacet.webSearch.active"
    static let rootKey = "tacet.webSearch.rootURL"

    // MARK: - On/off

    /// Is search on? It can be switched off with a single tap even while a server is
    /// configured.
    ///
    /// If the address is invalid/empty it returns `false` even when the flag is on:
    /// there is no "on but not working" intermediate state — the tool either enters the
    /// profile or it does not.
    static var isActive: Bool {
        get {
            guard UserDefaults.standard.bool(forKey: activeKey) else { return false }
            return rootURL != nil
        }
        set { UserDefaults.standard.set(newValue, forKey: activeKey) }
    }

    // MARK: - Root address

    /// The raw text the user typed (unvalidated) — the Settings field shows this.
    static var rawRoot: String {
        get {
            let saved = UserDefaults.standard.string(forKey: rootKey) ?? ""
            #if DEBUG
            // A developer convenience; this branch does not exist in the App Store build.
            if saved.isEmpty {
                return ProcessInfo.processInfo.environment["TACET_SEARX_URL"] ?? ""
            }
            #endif
            return saved
        }
        set { UserDefaults.standard.set(newValue, forKey: rootKey) }
    }

    /// The validated root address. `nil` if invalid — the caller never enters network code.
    static var rootURL: URL? { validate(rawRoot) }

    /// The address rule (same as mcp §3.1): `https://` only. Plain `http://` is
    /// accepted ONLY for local-network addresses — if the user is connecting to a box
    /// on their own network, forcing TLS is a pointless barrier, but a plaintext query
    /// leaving for the internet is unacceptable.
    static func validate(_ raw: String) -> URL? {
        let clean = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !clean.isEmpty,
              let url = URL(string: clean),
              let schema = url.scheme?.lowercased(),
              let host = url.host?.lowercased(), !host.isEmpty
        else { return nil }

        switch schema {
        case "https":
            return url
        case "http":
            return isLocalNetwork(host) ? url : nil
        default:
            return nil
        }
    }

    /// Is it a local-network host — loopback, `.local` (Bonjour) and the RFC1918 blocks.
    static func isLocalNetwork(_ host: String) -> Bool {
        if host == "localhost" || host == "::1" { return true }
        if host.hasSuffix(".local") { return true }

        let chunk = host.split(separator: ".").map(String.init)
        guard chunk.count == 4, let a = Int(chunk[0]), let b = Int(chunk[1]),
              chunk.allSatisfy({ Int($0) != nil }) else { return false }

        switch a {
        case 127: return true            // 127.0.0.0/8
        case 10: return true             // 10.0.0.0/8
        case 192: return b == 168        // 192.168.0.0/16
        case 172: return (16...31).contains(b)  // 172.16.0.0/12
        case 169: return b == 254        // 169.254.0.0/16 (link-local)
        default: return false
        }
    }
}
