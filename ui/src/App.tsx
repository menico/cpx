import { useCallback, useEffect, useState } from "react";
import { api, message } from "./api";
import type {
  AdoptionRow,
  DefaultSession as Session,
  BindingRow,
  CheckView,
  PlanView,
  ProfileDetail as Detail,
  ProfileRow,
} from "./api";
import { Adoptable } from "./Adoptable";
import { AskSheet, type Ask } from "./Ask";
import { Bindings } from "./Bindings";
import { DefaultSession } from "./DefaultSession";
import { ScriptEditor } from "./ScriptEditor";
import { Health } from "./Health";
import { PlanSheet } from "./PlanSheet";
import { ProfileDetail, SWATCHES } from "./ProfileDetail";

type Tab = "profiles" | "directories" | "health";

export function App() {
  const [ready, setReady] = useState<boolean | null>(null);
  const [profiles, setProfiles] = useState<ProfileRow[]>([]);
  const [bindings, setBindings] = useState<BindingRow[]>([]);
  const [checks, setChecks] = useState<CheckView[]>([]);
  const [adoptable, setAdoptable] = useState<AdoptionRow[]>([]);
  const [defaultSession, setDefaultSession] = useState<Session | null>(null);
  const [plan, setPlan] = useState<PlanView | null>(null);
  const [tab, setTab] = useState<Tab>("profiles");
  const [selected, setSelected] = useState<Detail | null>(null);
  const [showPlan, setShowPlan] = useState(false);
  const [editingBase, setEditingBase] = useState(false);
  const [ask, setAsk] = useState<Ask | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const initialised = await api.isInitialised();
      setReady(initialised);
      if (!initialised) return;

      const [rows, bound, health, pending, candidates, unmanaged] = await Promise.all([
        api.profiles(),
        api.bindings(),
        api.checks(),
        api.plan(),
        api.adoptionCandidates(),
        api.defaultSession(),
      ]);
      setProfiles(rows);
      setBindings(bound);
      setChecks(health);
      setPlan(pending);
      setAdoptable(candidates);
      setDefaultSession(unmanaged);
      setError(null);

      // Keep an open detail panel in step with the config it is showing.
      setSelected((current) =>
        current && rows.some((r) => r.name === current.row.name) ? current : null,
      );
    } catch (e) {
      setError(message(e));
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  // Reopen the detail panel against fresh data after any edit.
  useEffect(() => {
    if (!selected) return;
    const name = selected.row.name;
    api
      .profile(name)
      .then(setSelected)
      .catch(() => setSelected(null));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [profiles]);

  // The detail panel and the plan sheet both want the same room, so opening
  // one closes the other.
  useEffect(() => {
    if (selected) setShowPlan(false);
  }, [selected]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      if (selected) setSelected(null);
      else if (editingBase) setEditingBase(false);
      else if (showPlan) setShowPlan(false);
      else void api.hideWindow();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [selected, editingBase, showPlan]);

  async function run(work: () => Promise<unknown>) {
    setBusy(true);
    setError(null);
    try {
      await work();
      await refresh();
    } catch (e) {
      setError(message(e));
    } finally {
      setBusy(false);
    }
  }

  function addProfile() {
    setAsk({
      kind: "text",
      title: "New profile",
      detail: "The name becomes its command: work gives you claude-work.",
      placeholder: "work",
      submit: "Create",
      onSubmit: (name) => {
        setAsk(null);
        void run(async () => {
          await api.addProfile(name, "");
          // Give it the next unused identity colour straight away, so it is
          // distinguishable before anyone has chosen anything.
          const used = new Set(profiles.map((p) => p.color));
          const colour = SWATCHES.find((c) => !used.has(c)) ?? SWATCHES[0];
          await api.setField(name, "color", colour);
        });
      },
    });
  }

  async function adoptDirectory(candidate: AdoptionRow) {
    await run(() => api.adopt(candidate.dir, null));
  }

  async function bindDirectory() {
    const dir = await api.pickDirectory();
    if (!dir) return;

    if (profiles.length === 1) {
      await run(() => api.bind(profiles[0].name, dir));
      return;
    }
    setAsk({
      kind: "choose",
      title: "Which profile should this use?",
      detail: dir,
      options: profiles.map((profile) => ({
        id: profile.name,
        label: profile.name,
        hint: profile.account ?? profile.description,
        color: profile.color,
      })),
      onChoose: (name) => {
        setAsk(null);
        void run(() => api.bind(name, dir));
      },
    });
  }

  const pending = plan?.lines.length ?? 0;
  const failing = checks.filter((c) => c.severity === "error").length;
  const warning = checks.filter((c) => c.severity === "warning").length;

  if (ready === null) return <div className="app" />;

  if (!ready) {
    return (
      <div className="app">
        <div className="titlebar" />
        <div className="body">
          <div className="empty">
            <h2>cpx</h2>
            <p>
              Run several Claude accounts side by side. Each gets its own login, its own command,
              and can be bound to a directory.
            </p>
            <button className="btn primary" disabled={busy} onClick={() => run(() => api.initialise([]))}>
              Create configuration
            </button>
          </div>
          {error && <div className="error">{error}</div>}
        </div>
      </div>
    );
  }

  return (
    <div className="app">
      <div className="titlebar" />

      <div className="tabs" role="tablist">
        <button
          className="tab"
          role="tab"
          aria-selected={tab === "profiles"}
          onClick={() => setTab("profiles")}
        >
          profiles<span className="count">{profiles.length}</span>
        </button>
        <button
          className="tab"
          role="tab"
          aria-selected={tab === "directories"}
          onClick={() => setTab("directories")}
        >
          directories<span className="count">{bindings.length}</span>
        </button>
        <button
          className="tab"
          role="tab"
          aria-selected={tab === "health"}
          onClick={() => setTab("health")}
        >
          health
          {failing > 0 ? (
            <span className="dot fail" aria-label={`${failing} problems`} />
          ) : warning > 0 ? (
            <span className="dot warn" aria-label={`${warning} warnings`} />
          ) : (
            <span className="dot ok" aria-label="all clear" />
          )}
        </button>
      </div>

      <div className="body">
        {selected ? (
          <ProfileDetail
            detail={selected}
            onBack={() => setSelected(null)}
            onChanged={refresh}
            onError={setError}
            onAsk={setAsk}
          />
        ) : editingBase ? (
          <ScriptEditor
            profile={null}
            onClose={() => setEditingBase(false)}
            onError={setError}
          />
        ) : (
          <>
            {error && <div className="error">{error}</div>}

            {tab === "profiles" && (
              <>
                {profiles.length === 0 && adoptable.every((c) => c.taken) ? (
                  <div className="empty">
                    <h2>No profiles yet</h2>
                    <p>A profile is one Claude account, with its own login and its own command.</p>
                    <button className="btn primary" onClick={addProfile}>
                      Add a profile
                    </button>
                  </div>
                ) : (
                  profiles.map((profile) => (
                    <button
                      className="row"
                      key={profile.name}
                      style={{ ["--bar" as string]: profile.color ?? undefined }}
                      onClick={() =>
                        api
                          .profile(profile.name)
                          .then(setSelected)
                          .catch((e) => setError(message(e)))
                      }
                    >
                      <span className="bar" aria-hidden />
                      <span className="grow">
                        <span className="name">{profile.name}</span>
                        <span className="sub">
                          <span className="mono">{profile.command}</span>
                          {profile.model && <span>· {profile.model}</span>}
                          {profile.adopted && <span>· in place</span>}
                          {profile.description && (
                            <span className="trunc">· {profile.description}</span>
                          )}
                        </span>
                      </span>
                      <span className="right">
                        <span className="account">{profile.account ?? ""}</span>
                        <span className="state">
                          <span
                            className={`dot ${
                              !profile.applied ? "off" : profile.signedIn ? "ok" : "warn"
                            }`}
                            aria-hidden
                          />
                          <span>
                            {!profile.applied
                              ? "not built"
                              : profile.signedIn
                                ? "ready"
                                : "signed out"}
                          </span>
                        </span>
                      </span>
                    </button>
                  ))
                )}
                {defaultSession && (
                  <DefaultSession
                    session={defaultSession}
                    onChanged={refresh}
                    onError={setError}
                    onEdit={() => setEditingBase(true)}
                  />
                )}
                <Adoptable candidates={adoptable} onAdopt={adoptDirectory} />
              </>
            )}

            {tab === "directories" && (
              <Bindings
                bindings={bindings}
                profiles={profiles}
                onBind={bindDirectory}
                onUnbind={(path) => run(() => api.unbind(path))}
              />
            )}

            {tab === "health" && <Health checks={checks} />}
          </>
        )}
      </div>

      {ask && <AskSheet ask={ask} onCancel={() => setAsk(null)} />}

      <div className="footer">
        {showPlan && plan && pending > 0 && <PlanSheet plan={plan} />}
        <div className="footer-bar">
          <button className="btn small" onClick={addProfile}>
            New profile
          </button>
          <div className="grow" />
          {pending === 0 ? (
            <span className="state">Up to date</span>
          ) : (
            <>
              <button className="btn small quiet" onClick={() => setShowPlan(!showPlan)}>
                {showPlan ? "Hide changes" : `${pending} change${pending === 1 ? "" : "s"}`}
              </button>
              <button
                className="btn primary small"
                disabled={busy}
                onClick={() => run(() => api.apply(plan?.requiresForce ?? false, false))}
              >
                {plan?.requiresForce ? "Apply & back up" : "Apply"}
              </button>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
