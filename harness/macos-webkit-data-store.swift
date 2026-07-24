import CryptoKit
import Foundation

private let expectedBundleIdentifier = "com.clark.desktop.dev"
private let authStorageKey = "clark.auth.session"
private let settingsStorageKey = "clark-desktop:local-agent"
private let markerStorageKey = "clark-code:macos-qa-profile-marker"
private let maximumBootstrapBytes = 128 * 1024

private enum ToolFailure: Error, CustomStringConvertible {
    case message(String)

    var description: String {
        switch self {
        case let .message(message):
            return message
        }
    }
}

private struct AuthSession: Codable {
    struct User: Codable {
        let id: String
        let name: String
        let email: String
        let method: String?
    }

    struct Clark: Codable {
        let endpoint: String
        let token: String
    }

    let user: User
    let clark: Clark
}

private struct SeedPayload: Codable {
    let auth_session: AuthSession
    let cwd: String
    let model: String
    let marker: String
}

private func safeJSON(_ value: [String: Any]) -> String {
    guard
        JSONSerialization.isValidJSONObject(value),
        let data = try? JSONSerialization.data(
            withJSONObject: value,
            options: [.sortedKeys]
        ),
        let result = String(data: data, encoding: .utf8)
    else {
        return "{\"status\":\"failed\",\"error\":\"could not serialize safe result\"}"
    }
    return result
}

private func succeed(_ value: [String: Any]) -> Never {
    print(safeJSON(value))
    fflush(stdout)
    exit(0)
}

private func fail(_ message: String) -> Never {
    print(safeJSON([
        "status": "failed",
        "error": message,
        "credential_recorded": false,
    ]))
    fflush(stdout)
    exit(1)
}

private func jsStringLiteral(_ value: String) throws -> String {
    let data = try JSONSerialization.data(withJSONObject: [value])
    guard
        let encoded = String(data: data, encoding: .utf8),
        encoded.count >= 2
    else {
        throw ToolFailure.message("could not encode JavaScript string")
    }
    return String(encoded.dropFirst().dropLast())
}

private func fingerprint(_ value: String) -> String {
    SHA256.hash(data: Data(value.utf8))
        .prefix(8)
        .map { String(format: "%02x", $0) }
        .joined()
}

private func parseIdentifier(_ source: String) throws -> UUID {
    guard let identifier = UUID(uuidString: source) else {
        throw ToolFailure.message("data store identifier is not a UUID")
    }
    return identifier
}

private func validateBundleIdentity() throws {
    guard Bundle.main.bundleIdentifier == expectedBundleIdentifier else {
        throw ToolFailure.message("helper bundle identifier does not match Clark Code Dev")
    }
}

private func readSecureBootstrap(_ path: String) throws -> Data {
    let url = URL(fileURLWithPath: path)
    let values = try url.resourceValues(forKeys: [
        .isRegularFileKey,
        .isSymbolicLinkKey,
        .fileSizeKey,
    ])
    guard values.isRegularFile == true, values.isSymbolicLink != true else {
        throw ToolFailure.message("bootstrap must be a regular non-symlink file")
    }
    let attributes = try FileManager.default.attributesOfItem(atPath: path)
    let owner = (attributes[.ownerAccountID] as? NSNumber)?.uint32Value
    let permissions = (attributes[.posixPermissions] as? NSNumber)?.uint16Value
    guard owner == getuid() else {
        throw ToolFailure.message("bootstrap must be owned by the current user")
    }
    guard let permissions, permissions & 0o077 == 0 else {
        throw ToolFailure.message("bootstrap must not be accessible by group or other users")
    }
    guard let size = values.fileSize, size > 0, size <= maximumBootstrapBytes else {
        throw ToolFailure.message("bootstrap size is outside the allowed range")
    }
    return try Data(contentsOf: url, options: [.mappedIfSafe])
}

private func validatedSeedPayload(at path: String) throws -> SeedPayload {
    let payload = try JSONDecoder().decode(SeedPayload.self, from: readSecureBootstrap(path))
    let emailParts = payload.auth_session.user.email.lowercased().split(separator: "@")
    guard emailParts.count == 2, emailParts.last == "clarkslabs.com" else {
        throw ToolFailure.message("bootstrap account is not Clark-owned")
    }
    guard
        !payload.auth_session.user.id.isEmpty,
        !payload.auth_session.user.name.isEmpty
    else {
        throw ToolFailure.message("bootstrap account identity is incomplete")
    }
    guard
        payload.auth_session.clark.endpoint.hasPrefix("wss://"),
        payload.auth_session.clark.token.split(separator: ".").count == 3
    else {
        throw ToolFailure.message("bootstrap Clark session is malformed")
    }
    let project = URL(fileURLWithPath: payload.cwd).standardizedFileURL.path
    guard
        project.hasPrefix("/"),
        project.contains("/target/macos-qa-workspaces/"),
        FileManager.default.fileExists(atPath: project)
    else {
        throw ToolFailure.message("bootstrap project is outside the macOS QA workspace root")
    }
    guard payload.model == "clark-code:minimax_m3" else {
        throw ToolFailure.message("bootstrap model is not the bounded cheapest-paid route")
    }
    guard UUID(uuidString: payload.marker) != nil else {
        throw ToolFailure.message("bootstrap marker is not a UUID")
    }
    return payload
}

private func seed(identifier: UUID, bootstrapPath: String) throws -> Never {
    let payload = try validatedSeedPayload(at: bootstrapPath)
    let authData = try JSONEncoder().encode(payload.auth_session)
    guard let authJSON = String(data: authData, encoding: .utf8) else {
        throw ToolFailure.message("could not encode Clark session")
    }
    let settings: [String: Any] = [
        "cwd": payload.cwd,
        "model": payload.model,
        "reasoningEffort": "",
        "apiKey": "",
        "apiKeyOwner": "",
        "computerUseEnabled": false,
    ]
    let settingsData = try JSONSerialization.data(
        withJSONObject: settings,
        options: [.sortedKeys]
    )
    guard let settingsJSON = String(data: settingsData, encoding: .utf8) else {
        throw ToolFailure.message("could not encode Clark settings")
    }
    let authLiteral = try jsStringLiteral(authJSON)
    let settingsLiteral = try jsStringLiteral(settingsJSON)
    let markerLiteral = try jsStringLiteral(payload.marker)
    let script = """
    (() => {
      localStorage.setItem(\(try jsStringLiteral(authStorageKey)), \(authLiteral));
      localStorage.setItem(\(try jsStringLiteral(settingsStorageKey)), \(settingsLiteral));
      localStorage.setItem(\(try jsStringLiteral(markerStorageKey)), \(markerLiteral));
      return JSON.stringify({
        auth_present: Boolean(localStorage.getItem(\(try jsStringLiteral(authStorageKey)))),
        settings_present: Boolean(localStorage.getItem(\(try jsStringLiteral(settingsStorageKey)))),
        marker_present: localStorage.getItem(\(try jsStringLiteral(markerStorageKey))) === \(markerLiteral)
      });
    })()
    """
    return runWebKitEvaluation(identifier: identifier, script: script) { result in
        switch result {
        case let .success(source):
            guard
                let data = source.data(using: .utf8),
                let observed = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                observed["auth_present"] as? Bool == true,
                observed["settings_present"] as? Bool == true,
                observed["marker_present"] as? Bool == true
            else {
                fail("WebKit did not persist the QA bootstrap")
            }
            succeed([
                "status": "passed",
                "operation": "seed",
                "email_domain": "clarkslabs.com",
                "project_configured": true,
                "model_configured": true,
                "marker_present": true,
                "credential_recorded": false,
            ])
        case let .failure(error):
            fail(String(describing: error))
        }
    }
}

private func probe(
    identifier: UUID,
    expectedFingerprint: String,
    expectedProject: String,
    expectedModel: String,
    expectedMarker: String
) throws -> Never {
    let script = """
    (() => {
      let auth = null;
      let settings = null;
      try { auth = JSON.parse(localStorage.getItem(\(try jsStringLiteral(authStorageKey))) || "null"); } catch {}
      try { settings = JSON.parse(localStorage.getItem(\(try jsStringLiteral(settingsStorageKey))) || "null"); } catch {}
      const token = auth?.clark?.token || "";
      const owner = auth?.user?.id ? `id:${auth.user.id}` : "";
      const key = settings?.apiKey || "";
      let jwtFutureExpiry = false;
      let jwtIssuerPresent = false;
      try {
        let encoded = token.split(".")[1].replaceAll("-", "+").replaceAll("_", "/");
        encoded += "=".repeat((4 - (encoded.length % 4)) % 4);
        const payload = JSON.parse(atob(encoded));
        jwtFutureExpiry = Number.isInteger(payload.exp) && payload.exp > Math.floor(Date.now() / 1000);
        jwtIssuerPresent = typeof payload.iss === "string" && payload.iss.startsWith("https://");
      } catch {}
      return JSON.stringify({
        account_id: auth?.user?.id || "",
        email_domain: (auth?.user?.email || "").split("@").at(-1)?.toLowerCase() || "",
        endpoint_secure: (auth?.clark?.endpoint || "").startsWith("wss://"),
        jwt_shape: token.split(".").length === 3,
        jwt_future_expiry: jwtFutureExpiry,
        jwt_issuer_present: jwtIssuerPresent,
        project: settings?.cwd || "",
        model: settings?.model || "",
        provider_key_present: key.startsWith("ck_live_"),
        provider_key_owner_bound: Boolean(owner) && settings?.apiKeyOwner === owner,
        marker: localStorage.getItem(\(try jsStringLiteral(markerStorageKey))) || ""
      });
    })()
    """
    return runWebKitEvaluation(identifier: identifier, script: script) { result in
        switch result {
        case let .success(source):
            guard
                let data = source.data(using: .utf8),
                let observed = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
            else {
                fail("could not parse WebKit QA profile state")
            }
            let accountID = observed["account_id"] as? String ?? ""
            let accountBound = !accountID.isEmpty
                && fingerprint(accountID) == expectedFingerprint
            let workspace: [String: Any] = [
                "email_domain": observed["email_domain"] as? String ?? "",
                "account_bound": accountBound,
                "endpoint_secure": observed["endpoint_secure"] as? Bool ?? false,
                "jwt_shape": observed["jwt_shape"] as? Bool ?? false,
                "jwt_future_expiry": observed["jwt_future_expiry"] as? Bool ?? false,
                "jwt_issuer_present": observed["jwt_issuer_present"] as? Bool ?? false,
                "project_configured":
                    (observed["project"] as? String ?? "") == expectedProject,
                "model_configured":
                    (observed["model"] as? String ?? "") == expectedModel,
                "provider_key_present":
                    observed["provider_key_present"] as? Bool ?? false,
                "provider_key_owner_bound":
                    observed["provider_key_owner_bound"] as? Bool ?? false,
                "marker_present":
                    (observed["marker"] as? String ?? "") == expectedMarker,
            ]
            let passed = workspace.values.allSatisfy { value in
                if let boolean = value as? Bool {
                    return boolean
                }
                return value as? String == "clarkslabs.com"
            }
            if !passed {
                fail("isolated WebKit QA profile did not satisfy the workspace contract")
            }
            succeed([
                "status": "passed",
                "operation": "probe",
                "workspace": workspace,
                "credential_recorded": false,
            ])
        case let .failure(error):
            fail(String(describing: error))
        }
    }
}

@main
private struct MacosWebKitDataStoreTool {
    static func main() {
        do {
            try validateBundleIdentity()
            let arguments = CommandLine.arguments
            guard arguments.count >= 3 else {
                throw ToolFailure.message(
                    "usage: helper seed UUID BOOTSTRAP | probe UUID FINGERPRINT PROJECT MODEL MARKER"
                )
            }
            let operation = arguments[1]
            let identifier = try parseIdentifier(arguments[2])
            switch operation {
            case "seed":
                guard arguments.count == 4 else {
                    throw ToolFailure.message("seed expects UUID and bootstrap path")
                }
                try seed(identifier: identifier, bootstrapPath: arguments[3])
            case "probe":
                guard arguments.count == 7 else {
                    throw ToolFailure.message(
                        "probe expects UUID, fingerprint, project, model, and marker"
                    )
                }
                try probe(
                    identifier: identifier,
                    expectedFingerprint: arguments[3],
                    expectedProject: arguments[4],
                    expectedModel: arguments[5],
                    expectedMarker: arguments[6]
                )
            default:
                throw ToolFailure.message("unknown WebKit data-store operation")
            }
        } catch {
            fail(String(describing: error))
        }
    }
}
