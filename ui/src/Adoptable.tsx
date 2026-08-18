import type { AdoptionRow } from "./api";

interface Props {
  candidates: AdoptionRow[];
  onAdopt: (candidate: AdoptionRow) => void;
}

/** Directories that already exist and could be managed where they are.
 *  Only rendered when there are any, so it disappears once you are set up. */
export function Adoptable({ candidates, onAdopt }: Props) {
  const available = candidates.filter((c) => !c.taken);
  if (available.length === 0) return null;

  return (
    <div className="group adoptable">
      <h3>Already on this machine</h3>
      <p className="hint">
        Managed where they are. Nothing inside them changes, and each keeps the account it
        is already signed into.
      </p>
      {available.map((candidate) => (
        <div className="adopt-row" key={candidate.dir}>
          <div className="grow">
            <div className="name">{candidate.name}</div>
            <div className="sub">
              <span className="mono trunc" title={candidate.dir}>
                {candidate.dir}
              </span>
            </div>
            {candidate.keeps.length > 0 && (
              <div className="sub">keeps {candidate.keeps.join(", ")}</div>
            )}
          </div>
          <button className="btn small" onClick={() => onAdopt(candidate)}>
            Adopt
          </button>
        </div>
      ))}
    </div>
  );
}
