import type { BindingRow, ProfileRow } from "./api";

/** Said from the user's side of the screen: what is wrong and what to do. */
const HEALTH: Record<BindingRow["health"], string> = {
  healthy: "",
  directoryMissing: "Directory is gone",
  profileMissing: "Profile no longer exists",
  blockAbsent: "The .envrc no longer has its cpx block",
  blockEdited: "Edited by hand",
  notAllowed: "direnv has not allowed this .envrc",
};

interface Props {
  bindings: BindingRow[];
  profiles: ProfileRow[];
  onBind: () => void;
  onUnbind: (path: string) => void;
}

export function Bindings({ bindings, profiles, onBind, onUnbind }: Props) {
  if (profiles.length === 0) {
    return (
      <div className="empty">
        <h2>No profiles yet</h2>
        <p>Add a profile before binding a directory to one.</p>
      </div>
    );
  }

  if (bindings.length === 0) {
    return (
      <div className="empty">
        <h2>No directories bound</h2>
        <p>
          Bind a directory and everything you run there — including a plain <code>claude</code> —
          uses that profile.
        </p>
        <button className="btn primary" onClick={onBind}>
          Bind a directory…
        </button>
      </div>
    );
  }

  return (
    <>
      {bindings.map((binding) => (
        <div className="row" key={binding.path} style={{ ["--bar" as string]: binding.color ?? undefined }}>
          <span className="bar" aria-hidden />
          <div className="grow">
            <div className="name">{basename(binding.path)}</div>
            <div className="sub">
              <span className="mono trunc" title={binding.path}>
                {binding.path}
              </span>
            </div>
            {!binding.healthy && (
              <div className="sub">
                <span className="dot warn" aria-hidden />
                <span>{HEALTH[binding.health]}</span>
              </div>
            )}
          </div>
          <div className="right">
            <div className="account">{binding.profile}</div>
            <button className="btn quiet small" onClick={() => onUnbind(binding.path)}>
              Unbind
            </button>
          </div>
        </div>
      ))}
      <div className="group">
        <button className="btn" onClick={onBind}>
          Bind another directory…
        </button>
      </div>
    </>
  );
}

function basename(path: string): string {
  const parts = path.replace(/\/+$/, "").split("/");
  return parts[parts.length - 1] || path;
}
