import { useEffect, useRef, useState } from "react";
import type { StatuslineScript } from "./api";
import { api, message } from "./api";

interface Props {
  profile: string | null;
  onClose: () => void;
  onError: (message: string) => void;
}

/** Edits the script a statusline runs.
 *
 *  A script installed by a package manager gets overwritten on its next
 *  update, so editing one offers to take a copy first rather than quietly
 *  letting the work be lost. */
export function ScriptEditor({ profile, onClose, onError }: Props) {
  const [script, setScript] = useState<StatuslineScript | null>(null);
  const [draft, setDraft] = useState("");
  const [busy, setBusy] = useState(false);
  const [saved, setSaved] = useState(false);
  const area = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    api
      .statuslineScript(profile)
      .then((found) => {
        setScript(found);
        setDraft(found?.contents ?? "");
      })
      .catch((e) => onError(message(e)));
  }, [profile]);

  const dirty = script !== null && draft !== script.contents;

  async function save() {
    setBusy(true);
    try {
      await api.saveStatuslineScript(profile, draft);
      setScript((s) => (s ? { ...s, contents: draft } : s));
      setSaved(true);
      setTimeout(() => setSaved(false), 1600);
    } catch (e) {
      onError(message(e));
    } finally {
      setBusy(false);
    }
  }

  async function fork() {
    setBusy(true);
    try {
      await api.forkStatuslineScript(profile);
      const found = await api.statuslineScript(profile);
      setScript(found);
      setDraft(found?.contents ?? "");
    } catch (e) {
      onError(message(e));
    } finally {
      setBusy(false);
    }
  }

  if (script === null) {
    return (
      <div className="sheet">
        <div className="sheet-head">
          <button className="btn quiet small" onClick={onClose}>
            ‹ Back
          </button>
        </div>
        <div className="empty">
          <h2>Nothing to edit</h2>
          <p>
            This statusline does not run a script cpx can open — it may be a command on your
            PATH, or run through npx.
          </p>
        </div>
      </div>
    );
  }

  return (
    <div className="sheet">
      <div className="sheet-head">
        <button className="btn quiet small" onClick={onClose}>
          ‹ Back
        </button>
        <div className="grow" />
        {saved && <span className="state">Saved</span>}
        <button className="btn primary small" disabled={!dirty || busy} onClick={save}>
          Save
        </button>
      </div>

      <div className="editor-path" title={script.path}>
        {script.path}
      </div>

      {script.managedBy && (
        <div className="editor-warning">
          <div>
            Something else installs this file and will overwrite it on its next update, so
            edits here would be lost. Take a copy and this profile runs that instead.
          </div>
          <div className="mono">{script.managedBy}</div>
          <div>
            <button className="btn small" disabled={busy} onClick={fork}>
              Copy to this profile
            </button>
          </div>
        </div>
      )}

      <textarea
        ref={area}
        className="editor"
        spellCheck={false}
        value={draft}
        onChange={(e) => setDraft(e.target.value)}
        onKeyDown={(e) => {
          if ((e.metaKey || e.ctrlKey) && e.key === "s") {
            e.preventDefault();
            if (dirty) void save();
          }
        }}
      />

      <div className="editor-foot">
        <span>{draft.split("\n").length} lines</span>
        <div className="grow" />
        {dirty && (
          <button className="btn small" onClick={() => setDraft(script.contents)}>
            Revert
          </button>
        )}
      </div>
    </div>
  );
}
