import Foundation
import Contacts
import FoundationModels

// Looks a contact up by name and returns their phone number and email. Read-only.
struct ContactTool: TacetTool {
    let name = "contact"
    let description = "Looks up a contact by name and returns their phone number and email. Call this whenever the user asks for someone's number or email, in any language."

    weak var reporter: (any ToolReporter)?

    @Generable struct Arguments {
        @Guide(description: "First or last name of the contact to look up.")
        var name: String
    }

    func call(arguments: Arguments) async -> String {
        await runWithChip(icon: "person", runningText: L10n.searchingContacts, rawInput: arguments.name) {
            let store = CNContactStore()

            // Permission goes through a single gate (PermissionGate): if it was denied,
            // set the chip to needsPermission, do not throw.
            let permission = try await PermissionGate.contacts(store)
            if let cause = permission.toModel {
                return ToolOutcome(
                    chipText: L10n.contactsPermission,
                    state: .permissionRequired,
                    toModel: cause
                )
            }

            // Fetch the matching contacts.
            let keys: [CNKeyDescriptor] = [
                CNContactGivenNameKey as CNKeyDescriptor,
                CNContactFamilyNameKey as CNKeyDescriptor,
                CNContactPhoneNumbersKey as CNKeyDescriptor,
                CNContactEmailAddressesKey as CNKeyDescriptor
            ]
            let predicate = CNContact.predicateForContacts(matchingName: arguments.name)
            let contacts = try store.unifiedContacts(matching: predicate, keysToFetch: keys)

            // The address book was genuinely read: the session is tainted even if the result
            // is empty (mcp §5.6). The query itself is personal data too.
            if contacts.isEmpty {
                return await taintIfSucceeded(ToolOutcome(
                    chipText: L10n.contactsSearchedNone,
                    state: .readOk,
                    toModel: "no_contact_found"
                ))
            }

            // Reduces each contact to a single line: "First Last · number/email".
            func line(_ k: CNContact) -> String {
                let fullName = [k.givenName, k.familyName]
                    .filter { !$0.isEmpty }
                    .joined(separator: " ")
                var contactInfo: [String] = []
                if let phone = k.phoneNumbers.first?.value.stringValue, !phone.isEmpty {
                    contactInfo.append(phone)
                }
                if let email = k.emailAddresses.first?.value as String?, !email.isEmpty {
                    contactInfo.append(email)
                }
                let contactText = contactInfo.joined(separator: " ")
                let name = fullName.isEmpty ? "(no name)" : fullName
                return contactText.isEmpty ? name : "\(name) · \(contactText)"
            }

            let summary = contacts.prefix(5).map(line).joined(separator: "; ")
            let full = contacts.map(line).joined(separator: "\n")

            return await taintIfSucceeded(ToolOutcome(
                chipText: L10n.contactsSearched,
                state: .readOk,
                toModel: summary,
                rawOutput: full
            ))
        }
    }
}
