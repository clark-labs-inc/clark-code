import Foundation

private let expectedBundleIdentifier = "com.clark.desktop.dev"
private let settingsStorageKeyPrefix = "clark-desktop:local-agent:"
private let projectStorageKeyPrefix = "clark-desktop:project-context:"
private let remoteHostsStorageKeyPrefix = "clark-desktop:ssh-hosts:"
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

private struct SeedPayload: Codable {
    let cwd: String
    let model: String
    let marker: String
    let accountScope: String
    let remoteHost: String
    let remoteRoot: String
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
    let project = URL(fileURLWithPath: payload.cwd).standardizedFileURL.path
    guard
        project.hasPrefix("/"),
        project.contains("/target/macos-qa-workspaces/"),
        FileManager.default.fileExists(atPath: project)
    else {
        throw ToolFailure.message("bootstrap project is outside the macOS QA workspace root")
    }
    guard payload.model == "clark-code:free" else {
        throw ToolFailure.message("bootstrap model is not the included coding route")
    }
    guard payload.remoteHost == "nucleus" else {
        throw ToolFailure.message("bootstrap remote host is not the QA alias")
    }
    guard payload.remoteRoot == "/tmp/clark-code-registry-smoke" else {
        throw ToolFailure.message("bootstrap remote root is not the bounded QA fixture")
    }
    guard UUID(uuidString: payload.marker) != nil else {
        throw ToolFailure.message("bootstrap marker is not a UUID")
    }
    guard
        payload.accountScope == payload.accountScope.lowercased(),
        payload.accountScope.hasPrefix("id:"),
        payload.accountScope.count > 3,
        payload.accountScope.count <= 259
    else {
        throw ToolFailure.message("bootstrap account scope is invalid")
    }
    return payload
}

private func seed(identifier: UUID, bootstrapPath: String) throws -> Never {
    let payload = try validatedSeedPayload(at: bootstrapPath)
    let settings: [String: Any] = [
        "cwd": "",
        "model": payload.model,
        "reasoningEffort": "",
        "computerUseEnabled": false,
    ]
    let settingsData = try JSONSerialization.data(
        withJSONObject: settings,
        options: [.sortedKeys]
    )
    guard let settingsJSON = String(data: settingsData, encoding: .utf8) else {
        throw ToolFailure.message("could not encode Clark settings")
    }
    let settingsLiteral = try jsStringLiteral(settingsJSON)
    let accountScopeLiteral = try jsStringLiteral(payload.accountScope)
    let projectLiteral = try jsStringLiteral(payload.cwd)
    let markerLiteral = try jsStringLiteral(payload.marker)
    let remoteHosts = [[
        "id": "macos-qa-remote",
        "label": "Clark QA Remote",
        "host": payload.remoteHost,
        "remoteRoot": payload.remoteRoot,
    ]]
    let remoteHostsData = try JSONSerialization.data(
        withJSONObject: remoteHosts,
        options: [.sortedKeys]
    )
    guard let remoteHostsJSON = String(data: remoteHostsData, encoding: .utf8) else {
        throw ToolFailure.message("could not encode Clark remote hosts")
    }
    let remoteHostsLiteral = try jsStringLiteral(remoteHostsJSON)
    let script = """
    (() => {
      const encodedScope = encodeURIComponent(\(accountScopeLiteral));
      const settingsKey = \(try jsStringLiteral(settingsStorageKeyPrefix)) + encodedScope;
      const projectKey = \(try jsStringLiteral(projectStorageKeyPrefix)) + encodedScope;
      const remoteHostsKey = \(try jsStringLiteral(remoteHostsStorageKeyPrefix)) + encodedScope;
      localStorage.setItem(settingsKey, \(settingsLiteral));
      localStorage.setItem(projectKey, JSON.stringify({ cwd: \(projectLiteral) }));
      localStorage.setItem(remoteHostsKey, \(remoteHostsLiteral));
      localStorage.setItem(\(try jsStringLiteral(markerStorageKey)), \(markerLiteral));
      return JSON.stringify({
        credential_fields_absent: !/apiKey|apiKeyOwner|clarkToken|refreshToken|accessToken/i.test(
          JSON.stringify(Object.fromEntries(Object.entries(localStorage)))
        ),
        unscoped_settings_absent: localStorage.getItem("clark-desktop:local-agent") === null,
        settings_present: Boolean(localStorage.getItem(settingsKey)),
        project_present: Boolean(localStorage.getItem(projectKey)),
        remote_hosts_present: Boolean(localStorage.getItem(remoteHostsKey)),
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
                observed["credential_fields_absent"] as? Bool == true,
                observed["unscoped_settings_absent"] as? Bool == true,
                observed["settings_present"] as? Bool == true,
                observed["project_present"] as? Bool == true,
                observed["remote_hosts_present"] as? Bool == true,
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
    expectedProject: String,
    expectedModel: String,
    expectedMarker: String,
    expectedAccountScope: String,
    expectedRemoteHost: String,
    expectedRemoteRoot: String
) throws -> Never {
    let accountScopeLiteral = try jsStringLiteral(expectedAccountScope)
    let script = """
    (() => {
      const encodedScope = encodeURIComponent(\(accountScopeLiteral));
      const settingsKey = \(try jsStringLiteral(settingsStorageKeyPrefix)) + encodedScope;
      const projectKey = \(try jsStringLiteral(projectStorageKeyPrefix)) + encodedScope;
      const remoteHostsKey = \(try jsStringLiteral(remoteHostsStorageKeyPrefix)) + encodedScope;
      let settings = null;
      let project = null;
      let remoteHosts = [];
      try { settings = JSON.parse(localStorage.getItem(settingsKey) || "null"); } catch {}
      try { project = JSON.parse(localStorage.getItem(projectKey) || "null"); } catch {}
      try { remoteHosts = JSON.parse(localStorage.getItem(remoteHostsKey) || "[]"); } catch {}
      return JSON.stringify({
        credential_fields_absent: !/apiKey|apiKeyOwner|clarkToken|refreshToken|accessToken/i.test(
          JSON.stringify(Object.fromEntries(Object.entries(localStorage)))
        ),
        unscoped_settings_absent: localStorage.getItem("clark-desktop:local-agent") === null,
        project: project?.cwd || "",
        model: settings?.model || "",
        remote_host_configured: remoteHosts.length === 1
          && remoteHosts[0].host === \(try jsStringLiteral(expectedRemoteHost))
          && remoteHosts[0].remoteRoot === \(try jsStringLiteral(expectedRemoteRoot)),
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
            let workspace: [String: Any] = [
                "credential_fields_absent":
                    observed["credential_fields_absent"] as? Bool ?? false,
                "unscoped_settings_absent":
                    observed["unscoped_settings_absent"] as? Bool ?? false,
                "project_configured":
                    (observed["project"] as? String ?? "") == expectedProject,
                "model_configured":
                    (observed["model"] as? String ?? "") == expectedModel,
                "remote_host_configured":
                    observed["remote_host_configured"] as? Bool ?? false,
                "marker_present":
                    (observed["marker"] as? String ?? "") == expectedMarker,
            ]
            let passed = workspace.values.allSatisfy { value in
                if let boolean = value as? Bool {
                    return boolean
                }
                return false
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
                    "usage: helper seed UUID BOOTSTRAP | probe UUID PROJECT MODEL MARKER ACCOUNT_SCOPE REMOTE_HOST REMOTE_ROOT"
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
                guard arguments.count == 9 else {
                    throw ToolFailure.message(
                        "probe expects UUID, project, model, marker, account scope, remote host, and remote root"
                    )
                }
                try probe(
                    identifier: identifier,
                    expectedProject: arguments[3],
                    expectedModel: arguments[4],
                    expectedMarker: arguments[5],
                    expectedAccountScope: arguments[6],
                    expectedRemoteHost: arguments[7],
                    expectedRemoteRoot: arguments[8]
                )
            default:
                throw ToolFailure.message("unknown WebKit data-store operation")
            }
        } catch {
            fail(String(describing: error))
        }
    }
}
