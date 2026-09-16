import XCTest
@testable import CpxKit

final class ModelDecodingTests: XCTestCase {
    func testUsageReportDecodesWhatTheCommandPrints() throws {
        let json = """
        {"accounts":[{"profile":"work","provider":"claude","directory":"/p","command":"claude-work",
          "usage":{"provider":"claude","account":"me@x.io","plan":"Max 5x","windows":[
            {"id":"session","title":"Session","usedPercent":12.5,"windowMinutes":300,"resetsAt":"2026-09-16T12:00:00Z","resetsAtEpoch":1789560000}],
            "extraUsage":{"enabled":true,"usedUsd":1.5,"limitUsd":50,"utilization":3},"credits":null,"source":"oauth","fetchedAt":1789550000},
          "error":null,"message":null},
         {"profile":null,"provider":"codex","directory":"/c","command":"codex","usage":null,
          "error":{"kind":"notSignedIn"},"message":"not signed in"}]}
        """
        let report = try JSONDecoder().decode(UsageReport.self, from: Data(json.utf8))
        XCTAssertEqual(report.accounts.count, 2)
        XCTAssertEqual(report.accounts[0].id, "work")
        XCTAssertEqual(report.accounts[0].usage?.session?.remainingPercent, 87.5)
        XCTAssertEqual(report.accounts[0].usage?.session?.resetDate, Date(timeIntervalSince1970: 1_789_560_000))
        XCTAssertEqual(report.accounts[1].id, "@codex")
        XCTAssertTrue(report.accounts[1].isDefaultSession)
        XCTAssertEqual(report.accounts[1].error?.kind, "notSignedIn")
        XCTAssertTrue(report.accounts[1].error!.isLoginProblem)
    }

    func testProfileListDecodes() throws {
        let json = """
        {"profiles":[{"name":"work","provider":"codex","description":"","color":"#5c8dff","adopted":false,"directory":"/d",
          "applied":true,"authenticated":true,"account":"a@b","organization":null,"plan":"plus","expired":false,"source":"File","model":null,"command":"codex-work"}],
         "defaultSessions":[{"provider":"claude","directory":"/h/.claude","account":null,"signedIn":false,"isSource":true,"claimedBy":null}]}
        """
        let list = try JSONDecoder().decode(ProfileList.self, from: Data(json.utf8))
        XCTAssertEqual(list.profiles[0].provider, .codex)
        XCTAssertEqual(list.defaultSessions[0].provider, .claude)
    }

    func testResourceRowKnowsWhetherItHasAPatch() throws {
        let json = """
        [{"resource":"settings","mode":"merge","patch":{"model":"sonnet"},"isDirectory":false,"supportsMerge":true},
         {"resource":"skills","mode":"link","patch":null,"isDirectory":true,"supportsMerge":false}]
        """
        let rows = try JSONDecoder().decode([ResourceRow].self, from: Data(json.utf8))
        XCTAssertTrue(rows[0].hasPatch)
        XCTAssertFalse(rows[1].hasPatch)
    }

    func testCostReportDecodes() throws {
        let json = """
        {"accounts":[{"profile":"work","provider":"claude","directory":"/d","cost":{"provider":"claude",
          "days":[{"day":"2026-09-16","tokens":{"input":10,"output":5,"cacheRead":0,"cacheWrite":0,"cacheWrite1h":0},"costUsd":0.5,"partlyUnpriced":false}],
          "today":{"tokens":{"input":10,"output":5,"cacheRead":0,"cacheWrite":0},"costUsd":0.5,"partlyUnpriced":false},
          "last7Days":{"tokens":{"input":10,"output":5,"cacheRead":0,"cacheWrite":0},"costUsd":0.5,"partlyUnpriced":false},
          "last30Days":{"tokens":{"input":10,"output":5,"cacheRead":0,"cacheWrite":0},"costUsd":0.5,"partlyUnpriced":false},
          "models":[{"model":"claude-opus-5","tokens":{"input":10,"output":5,"cacheRead":0,"cacheWrite":0},"costUsd":0.5}],
          "unpricedModels":[],"filesScanned":1,"filesFromCache":0}}],"days":30}
        """
        let list = try JSONDecoder().decode(CostReportList.self, from: Data(json.utf8))
        XCTAssertEqual(list.accounts[0].cost.today.tokens.total, 15)
    }

    func testCLIErrorMessageDropsThePrefix() {
        let result = CLIResult(stdout: Data(), stderr: "cpx: no profile named `x`\n", exitCode: 1)
        XCTAssertEqual(CpxCLI.message(from: result), "no profile named `x`")
        let silent = CLIResult(stdout: Data(), stderr: "", exitCode: 3)
        XCTAssertEqual(CpxCLI.message(from: silent), "cpx exited with status 3")
    }
}

final class RefreshPolicyTests: XCTestCase {
    let now = Date(timeIntervalSince1970: 1_000_000)

    func testNeverOpenedMenuMeansLongIdle() {
        let decision = RefreshPolicy.nextDelay(for: RefreshInput(now: now))
        XCTAssertEqual(decision, RefreshDecision(delay: 1800, reason: .longIdle))
    }

    func testMenuAgeSelectsTheDelay() {
        func delay(_ ago: TimeInterval) -> RefreshDecision {
            RefreshPolicy.nextDelay(for: RefreshInput(now: now, lastMenuOpenAt: now.addingTimeInterval(-ago)))
        }
        XCTAssertEqual(delay(60).reason, .recentInteraction)
        XCTAssertEqual(delay(60).delay, 120)
        XCTAssertEqual(delay(30 * 60).reason, .warm)
        XCTAssertEqual(delay(2 * 3600).reason, .idle)
        XCTAssertEqual(delay(5 * 3600).reason, .longIdle)
        XCTAssertEqual(delay(-100).reason, .recentInteraction, "a clock moved back still reads as recent")
    }

    func testConstraintsWinOverEverything() {
        let decision = RefreshPolicy.nextDelay(for: RefreshInput(now: now, lastMenuOpenAt: now, lowPowerMode: true))
        XCTAssertEqual(decision.reason, .constrained)
        XCTAssertEqual(decision.delay, 1800)
    }

    func testCodingActivityCapsASlowCadence() {
        let idle = RefreshInput(now: now, lastMenuOpenAt: now.addingTimeInterval(-5 * 3600), lastCodingActivityAt: now.addingTimeInterval(-60))
        XCTAssertEqual(RefreshPolicy.nextDelay(for: idle), RefreshDecision(delay: 300, reason: .codingActivity))
        let recent = RefreshInput(now: now, lastMenuOpenAt: now, lastCodingActivityAt: now)
        XCTAssertEqual(RefreshPolicy.nextDelay(for: recent).reason, .recentInteraction, "activity never slows a faster cadence")
    }
}

final class QuotaAlertTests: XCTestCase {
    func testThresholdsAreSanitised() {
        XCTAssertEqual(QuotaThresholds.sanitized([]), [50, 20])
        XCTAssertEqual(QuotaThresholds.sanitized([20, 50, 50, 150, -3]), [99, 50, 20, 0])
        XCTAssertEqual(QuotaThresholds.active([0, 20]), [20])
    }

    func testADropAcrossOneThresholdFiresOnce() {
        var state = WindowAlertState()
        var outcome = QuotaAlertLogic.evaluate(remaining: 80, thresholds: [50, 20], state: state)
        XCTAssertNil(outcome.warning)
        state = outcome.state
        outcome = QuotaAlertLogic.evaluate(remaining: 45, thresholds: [50, 20], state: state)
        XCTAssertEqual(outcome.warning, 50)
        state = outcome.state
        outcome = QuotaAlertLogic.evaluate(remaining: 40, thresholds: [50, 20], state: state)
        XCTAssertNil(outcome.warning, "already fired for 50")
    }

    func testASteepDropFiresTheLowestThresholdOnly() {
        var state = WindowAlertState()
        state = QuotaAlertLogic.evaluate(remaining: 80, thresholds: [50, 20], state: state).state
        let outcome = QuotaAlertLogic.evaluate(remaining: 10, thresholds: [50, 20], state: state)
        XCTAssertEqual(outcome.warning, 20)
        XCTAssertEqual(outcome.state.fired, [50, 20])
    }

    func testRecoveryReArmsAThreshold() {
        var state = WindowAlertState()
        state = QuotaAlertLogic.evaluate(remaining: 80, thresholds: [50, 20], state: state).state
        state = QuotaAlertLogic.evaluate(remaining: 45, thresholds: [50, 20], state: state).state
        state = QuotaAlertLogic.evaluate(remaining: 90, thresholds: [50, 20], state: state).state
        XCTAssertTrue(state.fired.isEmpty)
        XCTAssertEqual(QuotaAlertLogic.evaluate(remaining: 30, thresholds: [50, 20], state: state).warning, 50)
    }

    func testFirstSampleBelowAThresholdFires() {
        let outcome = QuotaAlertLogic.evaluate(remaining: 10, thresholds: [50, 20], state: WindowAlertState())
        XCTAssertEqual(outcome.warning, 20)
    }

    func testSessionTransitions() {
        var state = WindowAlertState()
        state = QuotaAlertLogic.evaluate(remaining: 50, thresholds: [], state: state).state
        var outcome = QuotaAlertLogic.evaluate(remaining: 0, thresholds: [], state: state)
        XCTAssertEqual(outcome.transition, .depleted)
        outcome = QuotaAlertLogic.evaluate(remaining: 0, thresholds: [], state: outcome.state)
        XCTAssertEqual(outcome.transition, .none)
        outcome = QuotaAlertLogic.evaluate(remaining: 100, thresholds: [], state: outcome.state)
        XCTAssertEqual(outcome.transition, .restored)
        XCTAssertEqual(QuotaAlertLogic.evaluate(remaining: 0, thresholds: [], state: WindowAlertState()).transition, .depleted)
    }
}

final class StatusFeedTests: XCTestCase {
    func testStatuspageDocumentParses() {
        let json = """
        {"page":{"id":"x","name":"Anthropic","url":"https://status.claude.com","updated_at":"2026-09-16T10:00:00.123Z"},
         "status":{"indicator":"minor","description":"Partially Degraded Service"}}
        """
        let status = StatusFeed.parse(Data(json.utf8))
        XCTAssertEqual(status?.indicator, .minor)
        XCTAssertTrue(status!.indicator.isIncident)
        XCTAssertEqual(status?.description, "Partially Degraded Service")
        XCTAssertNotNil(status?.updatedAt)
        XCTAssertEqual(StatusFeed.parse(Data("{}".utf8))?.indicator, nil)
        XCTAssertEqual(StatusFeed.parse(Data(#"{"status":{"indicator":"weird"}}"#.utf8))?.indicator, .unknown)
    }
}

final class FormatTests: XCTestCase {
    func testCountdowns() {
        let now = Date(timeIntervalSince1970: 0)
        XCTAssertEqual(Format.countdown(to: now.addingTimeInterval(-5), from: now), "now")
        XCTAssertEqual(Format.countdown(to: now.addingTimeInterval(90), from: now), "in 1m")
        XCTAssertEqual(Format.countdown(to: now.addingTimeInterval(3 * 3600 + 5 * 60), from: now), "in 3h 05m")
        XCTAssertEqual(Format.countdown(to: now.addingTimeInterval(3 * 86400), from: now), "in 3d 0h")
    }

    func testMoneyAndTokens() {
        XCTAssertEqual(Format.money(12.345), "$12.35")
        XCTAssertEqual(Format.money(1234.5), "$1,235")
        XCTAssertEqual(Format.tokens(980), "980")
        XCTAssertEqual(Format.tokens(1500), "1.5k")
        XCTAssertEqual(Format.tokens(34_000), "34k")
        XCTAssertEqual(Format.tokens(2_500_000), "2.5M")
        XCTAssertEqual(Format.percent(130), "100%")
        XCTAssertEqual(Format.percent(12.4), "12%")
    }

    func testWindowLengths() {
        XCTAssertEqual(Format.windowLength(minutes: 300), "5 hours")
        XCTAssertEqual(Format.windowLength(minutes: 10080), "1 week")
        XCTAssertEqual(Format.windowLength(minutes: 1440), "1 day")
        XCTAssertEqual(Format.windowLength(minutes: 45), "45 min")
    }
}
