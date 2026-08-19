import { useEffect, useState } from "react";
import type { Statusline } from "./api";
import { api, message } from "./api";

interface Props {
  /** The profile to configure, or null for the default session. */
  profile: string | null;
  /** What the badge will read — the profile name, or "default". */
  label: string;
  onChanged: () => void;
  onError: (message: string) => void;
  /** Open the editor for the script this statusline runs. */
  onEdit: () => void;
}

/** Turns a profile badge on or off in front of the statusline already
 *  configured. The existing statusline script is never edited: the badge runs
 *  first and then hands the session straight to it. */
export function StatuslineControl({ profile, label, onChanged, onError, onEdit }: Props) {
  const [state, setState] = useState<Statusline | null>(null);
  const [busy, setBusy] = useState(false);

  const load = () => {
    api
      .statusline(profile)
      .then(setState)
      .catch((e) => onError(message(e)));
  };

  useEffect(load, [profile]);

  async function toggle() {
    if (!state) return;
    setBusy(true);
    try {
      if (state.badge) await api.clearStatusline(profile);
      else await api.setStatusline(profile, null, null);
      load();
      onChanged();
    } catch (e) {
      onError(message(e));
    } finally {
      setBusy(false);
    }
  }

  if (!state) return null;

  return (
    <>
      <div className="field">
        <label>Profile badge</label>
        <div className="value" style={{ fontFamily: "var(--sans)" }}>
          {state.badge ? "shown" : "not shown"}
        </div>
        <button className="btn small" disabled={busy} onClick={toggle}>
          {state.badge ? "Remove" : "Add"}
        </button>
      </div>

      {state.badge && (
        <div className="field">
          <label>Preview</label>
          <div className="value">
            <span className="badge-sample">● {label}</span>
            <span className="badge-rest"> │ your statusline</span>
          </div>
        </div>
      )}

      <div className="field">
        <label>{state.badge ? "In front of" : "Statusline"}</label>
        <div className="value wrap">{state.delegate ?? "none configured"}</div>
        {state.delegate && (
          <button className="btn small" onClick={onEdit}>
            Edit…
          </button>
        )}
      </div>

      {state.badge && state.needsApply && (
        <div className="field">
          <label />
          <div className="value" style={{ fontFamily: "var(--sans)", color: "var(--muted)" }}>
            Recorded in your config; takes effect on the next apply.
          </div>
        </div>
      )}
    </>
  );
}
