import type { PlanView } from "./api";

/** The same gutter glyphs `cpx apply --dry-run` prints, so the app and the
 *  terminal never describe the same change differently. */
const GUTTER: Record<string, string> = { safe: "", generated: "~", foreign: "!" };

export function PlanSheet({ plan }: { plan: PlanView }) {
  return (
    <div className="plan">
      {plan.lines.map((line, i) => (
        <div className={`plan-line ${line.risk}`} key={i}>
          <span className="gutter" aria-hidden>
            {GUTTER[line.risk]}
          </span>
          <span className="verb">{line.verb}</span>
          <span className="path">{line.target}</span>
        </div>
      ))}
      {plan.notes.map((note, i) => (
        <div className="plan-note" key={`n${i}`}>
          {note}
        </div>
      ))}
      {plan.requiresForce && (
        <div className="plan-note">
          Lines marked ! would replace files cpx did not write. Each one is backed up first.
        </div>
      )}
    </div>
  );
}
