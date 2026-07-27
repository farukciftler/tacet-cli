//
//  IntentError.swift
//  Tacet
//
//  THE single error type of the intent layer (intent-plan §5).
//
//  The ban on silent failure becomes concrete here: no `perform()` returns an empty
//  `.result()` "as if it had been done" when the work could not be done, and no `EntityQuery`
//  returns an empty array when it could not read. An empty array says "you have no data",
//  whereas the truth is "I could not read" — in Shortcuts the two cannot be told apart.
//

import Foundation
import AppIntents
import FoundationModels

nonisolated enum IntentError: Error, CustomLocalizedStringResourceConvertible {
    /// The model is unreachable. The text comes from `L10n` — this file PRODUCES no new
    /// string, and the user reads in Shortcuts exactly the same sentence as in the app.
    case modelBlocked(String)
    case emptyPrompt
    case storeMissing
    case noteSize
    case memoryFull
    case couldNotWrite
    case chatMissing
    case documentMissing
    case fileMissing
    case exportDisabled
    case locked
    case fileTooLarge

    var localizedStringResource: LocalizedStringResource {
        switch self {
        case .modelBlocked(let text):
            // An already-resolved String; it is wrapped as a literal so no new key is opened
            // in the catalog (the three messages are the same as `ModelService.blockMessage`).
            return LocalizedStringResource(stringLiteral: text)
        case .emptyPrompt:
            return "An empty prompt can’t be sent."
        case .storeMissing:
            return "Tacet’s store couldn’t be opened. Open the app once and try again."
        case .noteSize:
            return "A note must be between 10 and 160 characters and contain at least one keyword."
        case .memoryFull:
            return "Tacet’s memory is full (50 notes). Delete a few notes from the Memory board."
        case .couldNotWrite:
            return "Couldn’t save the note"
        case .chatMissing:
            return "That chat no longer exists."
        case .documentMissing:
            return "Tacet hasn’t made a document yet."
        case .fileMissing:
            return "That file no longer exists."
        case .exportDisabled:
            return "Tacet doesn’t give files to Shortcuts. To allow it, turn it on in Tacet > Settings > Shortcuts."
        case .locked:
            return "The device must be unlocked to read the file."
        case .fileTooLarge:
            return "This file is too large to give to Shortcuts (25 MB max)."
        }
    }

    // MARK: - Model pre-check

    /// Is the model reachable? If not, an error carrying THE SAME sentence AS INSIDE THE APP
    /// is returned; `nil` means the path is clear.
    ///
    /// Intents that need the model call this on the FIRST line of `perform()` and, if there is
    /// a block, throw without putting anything into the inbox: otherwise a prompt that cannot
    /// be answered would land there when the app opens.
    @MainActor
    static func modelBlock() -> IntentError? {
        switch SystemLanguageModel.default.availability {
        case .available:
            return nil
        case .unavailable(let cause):
            switch cause {
            case .modelNotReady:
                return .modelBlocked(L10n.modelPreparing)
            case .appleIntelligenceNotEnabled:
                return .modelBlocked(L10n.appleIntelligenceClosed)
            case .deviceNotEligible:
                return .modelBlocked(L10n.deviceNotEligible)
            @unknown default:
                return .modelBlocked(L10n.deviceNotEligible)
            }
        }
    }
}
