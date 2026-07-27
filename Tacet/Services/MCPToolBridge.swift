//
//  MCPToolBridge.swift
//  Tacet
//
//  The MCP tool bridge (mcp §5.2, §5.4, §5.5). Moved out of `ModelService.swift`
//  AS IT WAS — there is no behaviour change.
//

import Foundation
import FoundationModels

// MARK: - The MCP tool bridge (mcp §5.2, §5.4, §5.5)

/// The layer that turns a saved connection into working `MCPTool` instances and runs
/// the remote call. `MCPTool` never touches the network API, it only calls
/// `MCPInvoker`; the network is still in one place (`MCPClient`).
///
/// The client is ONE instance per connection: the MCP session id (`Mcp-Session-Id`)
/// lives inside the client, and building a new client on every call would mean a new
/// handshake on every call.
@MainActor
final class MCPToolBridge: MCPInvoker {

    private struct Endpoint: Equatable { let url: URL; let key: String? }

    private var endpoints: [UUID: Endpoint] = [:]
    private var clients: [UUID: MCPClient] = [:]

    /// For the §5.5 result processing — large output is put here without passing
    /// through the model.
    weak var dataStore: DataStore?

    /// The owner of the external side-effect flag (audit P0-3). The moment the remote
    /// call REACHES the server, `markSideEffect()` is called and retry closes for that
    /// turn.
    weak var executor: ToolExecutor?

    init() {}

    /// Saves the connection's address. If the address or the key changed the client is
    /// thrown away: a new server is never spoken to with the old session id.
    func save(identity: UUID, url: URL, key: String?) {
        let new = Endpoint(url: url, key: key)
        guard endpoints[identity] != new else { return }
        endpoints[identity] = new
        clients[identity] = nil
    }

    /// All connections are gone (deleted / never existed): release the clients.
    func forget() {
        endpoints.removeAll()
        clients.removeAll()
    }

    private func client(_ identity: UUID) -> MCPClient? {
        if let available = clients[identity] { return available }
        guard let endpoint = endpoints[identity] else { return nil }
        let new = MCPClient(url: endpoint.url, key: endpoint.key)
        clients[identity] = new
        return new
    }

    // MARK: - Tool setup

    /// Reads the schemas from the server and produces the tools that enter the session.
    ///
    /// The definition that goes to the model is NOT THE SERVER'S RAW DESCRIPTION but the
    /// summary cached at add time (§5.3) — a raw description can fill the 4096 window with
    /// a single tool. A tool that is not in the cache (newly added, not yet summarised) is
    /// skipped this turn; it arrives once the summary is refreshed.
    ///
    /// If the network is unreachable an empty array is returned — the connection profile
    /// cannot be selected and today's behaviour continues. No invented tool is produced.
    func setUpTools(connectionID: UUID,
                    name: String,
                    summaries: [ToolSummary],
                    pool: Int,
                    deviceData: DeviceDataSetting,
                    gate: (any ApprovalGate)?,
                    reporter: (any ToolReporter)?) async -> [MCPTool] {
        guard let client = client(connectionID), !summaries.isEmpty else { return [] }
        guard let specs = try? await client.tools() else { return [] }

        var summaryTable: [String: String] = [:]
        for summary in summaries where !summary.isUnsupported { summaryTable[summary.name] = summary.summary }

        // The server order is preserved (deterministic), anything not in the cache is
        // eliminated, and the POOL cap is applied BEFORE the conversion: there is no point
        // converting 200 schemas on a 200-tool server. The pool is DELIBERATELY wider than
        // the session slot (6): which tools fill the slot is no longer decided here but by
        // `ToolRelevance`, which looks at the user's request in that turn (P1-6), and for
        // that it needs a pool to choose from. Had the pool also been 6, the relevance
        // ordering could do no more than reshuffle the server's first 6 among themselves.
        // Classification comes BEFORE the cap: on a 200-tool server the first 6 tools may
        // be `run_command`, `delete_file`, and in that case pure server order fills the
        // session with destructive tools and leaves the read-only ones out. Stable sorting
        // (tie-breaking with `enumerated`) keeps the server order within a class, so the
        // behaviour is still deterministic.
        let filtered = specs.filter { summaryTable[$0.name] != nil }
        let classes = filtered.map {
            SideEffectClass.classify(name: $0.name,
                                     summary: summaryTable[$0.name] ?? "",
                                     readOnlyHint: $0.readOnlyHint,
                                     destructiveHint: $0.destructiveHint)
        }
        let candidates = zip(filtered, classes).enumerated()
            .sorted { left, right in
                let leftDestructive = left.element.1.requiresApproval
                let rightDestructive = right.element.1.requiresApproval
                if leftDestructive != rightDestructive { return !leftDestructive }
                return left.offset < right.offset
            }
            .map(\.element)
            .prefix(pool)

        // Name collisions are resolved at the collection level (P2-9). "get-user" and
        // "get_user" both reduced to `get_user` and the model did not know which of the
        // two it was calling; `resolveNames` gives them DIFFERENT names without disturbing
        // the order.
        let resolvedNames = MCPTool.resolveNames(candidates.map { (remoteName: $0.0.name, server: name) })

        var tools: [MCPTool] = []
        for (order, (spec, sideEffect)) in candidates.enumerated() {
            // The schema is converted at run time; a tool that cannot be converted is
            // SKIPPED (§5.2) — better no tool at all than producing wrong arguments.
            guard let schema = try? MCPSchemaConverter.convert(spec: Self.toSpec(spec)) else { continue }
            tools.append(MCPTool(connectionID: connectionID,
                                    connectionName: name,
                                    remoteName: spec.name,
                                    summary: summaryTable[spec.name] ?? "",
                                    parameters: schema,
                                    invoker: self,
                                    deviceData: deviceData,
                                    sideEffect: sideEffect,
                                    gate: gate,
                                    reporter: reporter,
                                    resolvedName: resolvedNames[order]))
        }
        return tools
    }

    /// The client spec → the raw spec the schema conversion expects.
    /// A tool without a schema = a tool without arguments: an empty object schema is
    /// given, and the tool is not dropped.
    private static func toSpec(_ spec: MCPClient.ToolSpec) -> MCPToolSpec {
        let emptyObject = Data(#"{"type":"object","properties":{}}"#.utf8)
        var data = emptyObject
        if let schema = spec.schema, case .object = schema,
           let encoded = try? JSONEncoder().encode(schema) {
            data = encoded
        }
        return MCPToolSpec(name: spec.name, description: spec.description, inputSchemaJSON: data)
    }

    // MARK: - The remote call (MCPInvoker)

    /// The approval gate was passed BEFORE this call, inside `MCPTool.call`; everything
    /// that arrives here is what the user saw.
    func invoke(connectionID: UUID, toolName: String, argumentsJSON: String) async throws -> MCPOutcome {
        guard let client = client(connectionID) else {
            throw MCPClient.MCPError.unreachable
        }
        // The model produces JSON that fits the schema; even so we do not invent input we
        // cannot parse — we fall back to an argument-less call.
        let arguments = JSONValue.parse(argumentsJSON) ?? .object([:])
        let (text, isError) = try await client.callTool(name: toolName, arguments: arguments)

        // THIS IS THE SINGLE POINT OF P0-3. The call RETURNED means the request reached
        // the server and the server processed it — an issue may have been opened, a record
        // written, an email sent. After this, sending the SAME prompt a second time in this
        // turn produces an irreversible repeat.
        //
        // `isError` DOES NOT and MUST NOT make a difference: in MCP, `isError` is the
        // server's own comment about the tool's result, not proof that the operation never
        // happened ("the issue was opened but field validation failed" also returns
        // isError). Only the `throw` path (a transport error) does not set the flag,
        // because there the request never reached the server.
        executor?.markSideEffect()

        // §5.5: the raw output does not enter the model; a summary + sourceRef go, and the
        // whole of it stays in the chip.
        let processed = ConnectionService.processOutcome(text, toolName: toolName, dataStore: dataStore)
        let body = processed.toModel.trimmingCharacters(in: .whitespacesAndNewlines)

        if isError {
            // The server's OWN error (the command failed) — not a transport error.
            // The model reads it and reports it; the chip says "returned an error" too, so
            // it is not passed over silently.
            return MCPOutcome(
                chipDetail: String(localized: "\(toolName) returned an error"),
                toModel: body.isEmpty
                    ? "remote_tool_error: the tool failed on the user's server without a message. Say this in one sentence."
                    : "remote_tool_error: \(body)",
                rawOutput: processed.rawOutput)
        }
        return MCPOutcome(
            chipDetail: String(localized: "\(toolName) done"),
            toModel: body.isEmpty
                ? "remote_tool_empty: the tool ran but returned nothing. Say this in one sentence; do not invent a result."
                : body,
            rawOutput: processed.rawOutput)
    }
}
