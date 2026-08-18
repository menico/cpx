import type { CheckView } from "./api";

const TONE = { ok: "ok", warning: "warn", error: "fail" } as const;

export function Health({ checks }: { checks: CheckView[] }) {
  const findings = checks.filter((c) => c.severity !== "ok");

  if (findings.length === 0) {
    return (
      <div className="empty">
        <h2>All clear</h2>
        <p>{checks.length} checks passed.</p>
      </div>
    );
  }

  return (
    <>
      {findings.map((check, i) => (
        <div className="check" key={i}>
          <span className={`dot ${TONE[check.severity]} icon`} aria-hidden />
          <div>
            <div className="cname">{check.name}</div>
            <div className="cdetail">{check.detail}</div>
            {check.remedy && (
              <div className="cremedy">
                <code>{check.remedy}</code>
              </div>
            )}
          </div>
        </div>
      ))}
    </>
  );
}
