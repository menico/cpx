import type { DefaultSession as Session } from "./api";

/** The directory a plain `claude` uses.
 *
 *  cpx treats it as the source profiles inherit from rather than as a profile,
 *  so it is shown and never managed — but leaving it out would mean the app
 *  lists fewer accounts than the machine has. */
export function DefaultSession({ session }: { session: Session }) {
  // Already listed as a profile of its own.
  if (session.claimedBy) return null;

  return (
    <div className="group unmanaged">
      <h3>Not managed by cpx</h3>
      <div className="unmanaged-row">
        <div className="grow">
          <div className="name">claude</div>
          <div className="sub">
            <span className="mono trunc" title={session.dir}>
              {session.dir}
            </span>
          </div>
          <div className="sub wrap">
            {session.isSource
              ? "A plain claude uses it. Your profiles inherit from it."
              : "A plain claude uses it."}
          </div>
        </div>
        <div className="right">
          <div className="account">{session.account ?? ""}</div>
          <div className="state">
            <span className={`dot ${session.signedIn ? "ok" : "off"}`} aria-hidden />
            <span>{session.signedIn ? "signed in" : "signed out"}</span>
          </div>
        </div>
      </div>
    </div>
  );
}
