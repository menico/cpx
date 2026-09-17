// Driving the `cpx` command.
//
// The app makes no decision the command does not: every read is a
// `cpx … --json`, every write is the matching subcommand, and errors reach
// the interface as the command's own words.

import Foundation

public struct CLIError: Error, LocalizedError, Sendable {
    public var message: String
    public var exitCode: Int32

    public var errorDescription: String? { message }

    public init(message: String, exitCode: Int32 = 1) {
        self.message = message
        self.exitCode = exitCode
    }
}

public struct CLIResult: Sendable {
    public var stdout: Data
    public var stderr: String
    public var exitCode: Int32
}

/// Runs the command. Where the executable lives and what environment it
/// gets are the only things the app decides.
public final class CpxCLI: @unchecked Sendable {
    public let executable: URL
    public let environment: [String: String]

    public init(executable: URL, environment: [String: String]) {
        self.executable = executable
        self.environment = environment
    }

    /// The bundled command next to the app's own executable, an explicit
    /// `CPX_CLI` override, or one on the user's PATH.
    public static func locate(bundleExecutable: URL?, environment: [String: String]) -> URL? {
        if let override = environment["CPX_CLI"], !override.isEmpty {
            return URL(fileURLWithPath: override)
        }
        if let bundled = bundleExecutable?.deletingLastPathComponent().appendingPathComponent("cpx"),
           FileManager.default.isExecutableFile(atPath: bundled.path) {
            return bundled
        }
        for dir in (environment["PATH"] ?? "").split(separator: ":") {
            let candidate = URL(fileURLWithPath: String(dir)).appendingPathComponent("cpx")
            if FileManager.default.isExecutableFile(atPath: candidate.path) {
                return candidate
            }
        }
        return nil
    }

    /// The user's real PATH, from their login shell. A Finder-launched app
    /// inherits a stub one, which would hide `claude`, `codex` and `direnv`.
    public static func loginPath(shell: String? = nil) -> String? {
        let shell = shell ?? ProcessInfo.processInfo.environment["SHELL"] ?? "/bin/zsh"
        let process = Process()
        process.executableURL = URL(fileURLWithPath: shell)
        process.arguments = ["-lic", "printf %s \"$PATH\""]
        var env = ProcessInfo.processInfo.environment
        env.removeValue(forKey: "PATH")
        process.environment = env
        let pipe = Pipe()
        process.standardOutput = pipe
        process.standardError = FileHandle.nullDevice
        do {
            try process.run()
        } catch {
            return nil
        }
        let data = pipe.fileHandleForReading.readDataToEndOfFile()
        process.waitUntilExit()
        let text = String(decoding: data, as: UTF8.self)
        let last = text.split(separator: "\n").last.map(String.init)?.trimmingCharacters(in: .whitespaces) ?? ""
        return last.contains("/") ? last : nil
    }

    /// Run `cpx` with `arguments`, off the calling thread.
    public func run(_ arguments: [String], stdin: String? = nil) async throws -> CLIResult {
        let executable = self.executable
        let environment = self.environment
        return try await withCheckedThrowingContinuation { continuation in
            DispatchQueue.global(qos: .userInitiated).async {
                let process = Process()
                process.executableURL = executable
                process.arguments = arguments
                process.environment = environment
                let out = Pipe()
                let err = Pipe()
                process.standardOutput = out
                process.standardError = err
                if let stdin {
                    let input = Pipe()
                    process.standardInput = input
                    input.fileHandleForWriting.write(Data(stdin.utf8))
                    try? input.fileHandleForWriting.close()
                } else {
                    process.standardInput = FileHandle.nullDevice
                }
                do {
                    try process.run()
                } catch {
                    continuation.resume(throwing: CLIError(message: "could not run \(executable.path): \(error.localizedDescription)"))
                    return
                }
                // Drain both pipes before waiting, or a chatty command blocks on a full pipe.
                let stdout = out.fileHandleForReading.readDataToEndOfFile()
                let stderr = err.fileHandleForReading.readDataToEndOfFile()
                process.waitUntilExit()
                continuation.resume(returning: CLIResult(
                    stdout: stdout,
                    stderr: String(decoding: stderr, as: UTF8.self),
                    exitCode: process.terminationStatus))
            }
        }
    }

    /// Run a `--json` command and decode its output.
    ///
    /// `cpx doctor` exits non-zero when something is broken while still
    /// printing its report, so a failure is only a failure when there is no
    /// JSON to read.
    public func json<T: Decodable>(_ type: T.Type, _ arguments: [String], stdin: String? = nil) async throws -> T {
        let result = try await run(["--json"] + arguments, stdin: stdin)
        if let decoded = try? JSONDecoder().decode(T.self, from: result.stdout) {
            return decoded
        }
        if result.exitCode != 0 {
            throw CLIError(message: Self.message(from: result), exitCode: result.exitCode)
        }
        do {
            return try JSONDecoder().decode(T.self, from: result.stdout)
        } catch {
            throw CLIError(message: "cpx \(arguments.joined(separator: " ")) answered something unexpected: \(error)")
        }
    }

    /// Run a mutating command, caring only that it succeeded.
    @discardableResult
    public func call(_ arguments: [String], stdin: String? = nil) async throws -> CLIResult {
        let result = try await run(["--json"] + arguments, stdin: stdin)
        if result.exitCode != 0 {
            throw CLIError(message: Self.message(from: result), exitCode: result.exitCode)
        }
        return result
    }

    static func message(from result: CLIResult) -> String {
        let text = result.stderr.trimmingCharacters(in: .whitespacesAndNewlines)
        let stripped = text.hasPrefix("cpx: ") ? String(text.dropFirst(5)) : text
        return stripped.isEmpty ? "cpx exited with status \(result.exitCode)" : stripped
    }
}

// MARK: - Commands

public extension CpxCLI {
    func paths() async throws -> Paths { try await json(Paths.self, ["paths"]) }
    func initialise() async throws { try await call(["init"]) }
    func list() async throws -> ProfileList { try await json(ProfileList.self, ["list"]) }
    func show(_ name: String) async throws -> ProfileDetail { try await json(ProfileDetail.self, ["show", name]) }
    func plan() async throws -> Plan { try await json(Plan.self, ["status"]) }

    func apply(force: Bool, sync: Bool) async throws -> ApplyResult {
        var args = ["apply"]
        if force { args.append("--force") }
        if sync { args.append("--sync") }
        return try await json(ApplyResult.self, args)
    }

    func bindings() async throws -> [BindingRow] { try await json([BindingRow].self, ["bindings"]) }
    func bind(profile: String, dir: String) async throws { try await call(["bind", profile, dir]) }

    func unbind(dir: String, provider: Provider?) async throws {
        var args = ["unbind", dir]
        if let provider { args += ["--provider", provider.rawValue] }
        try await call(args)
    }

    func checks() async throws -> [Check] { try await json([Check].self, ["doctor"]) }

    func addProfile(name: String, provider: Provider, description: String = "") async throws {
        try await call(["profile", "add", name, "--provider", provider.rawValue, "--description", description])
    }

    func removeProfile(_ name: String) async throws { try await call(["profile", "rm", name]) }
    func cloneProfile(from: String, to: String) async throws { try await call(["clone", from, to]) }

    func setField(profile: String, field: String, value: String?) async throws {
        var args = ["profile", "set", profile, field]
        if let value, !value.isEmpty { args.append(value) }
        try await call(args)
    }

    func setResource(profile: String, resource: String, mode: String) async throws {
        try await call(["profile", "resource", profile, resource, mode])
    }

    func adoptionCandidates() async throws -> [AdoptionRow] { try await json([AdoptionRow].self, ["adopt"]) }
    func adopt(dir: String) async throws { try await call(["adopt", dir]) }

    func statuslines() async throws -> [StatuslineRow] { try await json([StatuslineRow].self, ["statusline", "show"]) }

    private func statuslineTarget(_ profile: String?) -> [String] {
        profile.map { [$0] } ?? ["--base"]
    }

    func setStatusline(profile: String?) async throws { try await call(["statusline", "set"] + statuslineTarget(profile)) }
    func clearStatusline(profile: String?) async throws { try await call(["statusline", "clear"] + statuslineTarget(profile)) }

    func statuslineScript(profile: String?) async throws -> ScriptFile? {
        try await json(ScriptFile?.self, ["statusline", "script"] + statuslineTarget(profile))
    }

    func saveStatuslineScript(profile: String?, contents: String) async throws {
        try await call(["statusline", "script"] + statuslineTarget(profile) + ["--save", "-"], stdin: contents)
    }

    func forkStatuslineScript(profile: String?) async throws {
        try await call(["statusline", "script"] + statuslineTarget(profile) + ["--fork"])
    }

    func skills(profile: String) async throws -> SkillInventory { try await json(SkillInventory.self, ["skills", "list", profile]) }

    func setSkillEnabled(profile: String, skill: String, enabled: Bool) async throws {
        try await call(["skills", enabled ? "enable" : "disable", profile, skill])
    }

    func removeSkill(profile: String, skill: String) async throws { try await call(["skills", "remove", profile, skill]) }

    func setPluginEnabled(profile: String, key: String, enabled: Bool) async throws -> PluginChange {
        try await json(PluginChange.self, ["skills", "plugin", profile, key, enabled ? "--on" : "--off"])
    }

    /// `prompt` false forbids a Keychain dialog: logins cpx may not read
    /// yet come back as `keychainNeedsPermission` instead.
    func usage(accounts: [String] = [], prompt: Bool, keepLogins: Bool) async throws -> UsageReport {
        var args = ["usage"]
        if !prompt { args.append("--no-prompt") }
        if keepLogins { args.append("--keep-logins") }
        return try await json(UsageReport.self, args + accounts)
    }

    /// Delete the copies `--keep-logins` made.
    func forgetLogins() async throws { try await call(["usage", "--forget-logins"]) }

    func cost(accounts: [String] = [], days: Int = 30) async throws -> CostReportList {
        try await json(CostReportList.self, ["cost", "--days", String(days)] + accounts)
    }
}
